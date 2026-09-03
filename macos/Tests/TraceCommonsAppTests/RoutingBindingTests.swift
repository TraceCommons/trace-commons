import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// How `SettingsView`'s routing card is wired to the surface underneath it.
///
/// `RoutingSurfaceTests` proves the mapping, `RoutingSurfaceExportTests`
/// proves the words, and `RoutingCallTests` proves the bytes. None of the
/// three can see the layer between them and a contributor: which property
/// each control is bound to. A card that read `form.on` to decide a tool's
/// word, or disabled the port field on the wrong sense of the switch, would
/// pass all three suites and ship the defect this surface was rebuilt to
/// remove.
///
/// A SwiftUI `body` holding `@State` and an `@EnvironmentObject` cannot be
/// built, rendered or reflected outside a running window, so these assert
/// against the view's own source. That is a real limitation and worth
/// naming: they catch a binding pointed at the wrong property, and they do
/// not catch a layout that never puts the control on screen. They are
/// written to fail loudly rather than silently -- every locator below
/// reports the text it was looking in when it does not find what it needs,
/// so a refactor that moves this card produces a failure to fix and not a
/// test that quietly stops asserting.

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

private enum RoutingCard {
    /// `.../macos/Tests/TraceCommonsAppTests/RoutingBindingTests.swift`
    static let viewPath = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // TraceCommonsAppTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .appendingPathComponent("Sources/TraceCommonsApp/Views/SettingsView.swift")

    /// The `routing` computed property's body, braces matched.
    static func body(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration("private var routing: some View {", file: file, line: line)
    }

    /// The `routingState(copy:)` helper's body.
    static func stateBody(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration("private func routingState(copy: RoutingCopy) -> some View {", file: file, line: line)
    }

    /// The `RoutingTone` -> `TC.Tone` bridge.
    static func toneBridge(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration("private func tone(_ tone: RoutingTone) -> TC.Tone {", file: file, line: line)
    }

    /// The source between `signature` and the brace that closes it.
    ///
    /// Braces inside comments and string literals are not counted, because
    /// this card carries both and a naive scan would end the body in the
    /// middle of a doc comment.
    static func declaration(
        _ signature: String, file: StaticString = #filePath, line: UInt = #line
    ) -> String? {
        guard let text = try? String(contentsOf: viewPath, encoding: .utf8) else {
            XCTFail("could not read \(viewPath.path)", file: file, line: line)
            return nil
        }
        guard let start = text.range(of: signature) else {
            XCTFail("\(viewPath.lastPathComponent) no longer declares `\(signature)`", file: file, line: line)
            return nil
        }
        var depth = 1
        var index = start.upperBound
        var inString = false
        var inLineComment = false
        while index < text.endIndex {
            let character = text[index]
            let next = text.index(after: index)
            if inLineComment {
                if character == "\n" { inLineComment = false }
            } else if inString {
                if character == "\\" {
                    index = next < text.endIndex ? text.index(after: next) : text.endIndex
                    continue
                }
                if character == "\"" { inString = false }
            } else if character == "/", next < text.endIndex, text[next] == "/" {
                inLineComment = true
            } else if character == "\"" {
                inString = true
            } else if character == "{" {
                depth += 1
            } else if character == "}" {
                depth -= 1
                if depth == 0 { return String(text[start.upperBound..<index]) }
            }
            index = next
        }
        XCTFail("the body of `\(signature)` is unterminated", file: file, line: line)
        return nil
    }

    /// The source between two markers, both required to be present.
    static func region(
        of body: String, from opening: String, to closing: String,
        file: StaticString = #filePath, line: UInt = #line
    ) -> String? {
        guard let start = body.range(of: opening) else {
            XCTFail("the routing card no longer contains `\(opening)`", file: file, line: line)
            return nil
        }
        guard let end = body.range(of: closing, range: start.upperBound..<body.endIndex) else {
            XCTFail("no `\(closing)` follows `\(opening)` on the routing card", file: file, line: line)
            return nil
        }
        return String(body[start.upperBound..<end.lowerBound])
    }


    /// The argument of the first `.disabled(` that follows `marker`, braces
    /// and parens matched. The argument is what the assertion is about:
    /// `.disabled(form.on)` is present and is the inversion that would ship
    /// exactly the wrong two fields live.
    static func disabledArgument(
        after marker: String, in body: String,
        file: StaticString = #filePath, line: UInt = #line
    ) -> String? {
        guard let anchor = body.range(of: marker) else {
            XCTFail("the routing card no longer contains `\(marker)`", file: file, line: line)
            return nil
        }
        guard let call = body.range(of: ".disabled(", range: anchor.upperBound..<body.endIndex) else {
            XCTFail("nothing after `\(marker)` is gated at all", file: file, line: line)
            return nil
        }
        var depth = 1
        var index = call.upperBound
        while index < body.endIndex {
            if body[index] == "(" { depth += 1 }
            if body[index] == ")" {
                depth -= 1
                if depth == 0 { return String(body[call.upperBound..<index]) }
            }
            index = body.index(after: index)
        }
        XCTFail("the `.disabled(` after `\(marker)` is unterminated", file: file, line: line)
        return nil
    }

    static func occurrences(of needle: String, in haystack: String) -> Int {
        guard !needle.isEmpty else { return 0 }
        var count = 0
        var index = haystack.startIndex
        while let found = haystack.range(of: needle, range: index..<haystack.endIndex) {
            count += 1
            index = found.upperBound
        }
        return count
    }

    /// Every string literal in `source`, with its `\(...)` holes removed and
    /// with `//` comments skipped -- a comment is not something a
    /// contributor reads.
    static func stringLiterals(in source: String) -> [String] {
        var literals: [String] = []
        var current = ""
        var inString = false
        var inLineComment = false
        var interpolationDepth = 0
        var index = source.startIndex
        while index < source.endIndex {
            let character = source[index]
            let next = source.index(after: index)
            if inLineComment {
                if character == "\n" { inLineComment = false }
            } else if inString {
                if character == "\\", next < source.endIndex, source[next] == "(" {
                    interpolationDepth = 1
                    index = source.index(after: next)
                    while index < source.endIndex, interpolationDepth > 0 {
                        if source[index] == "(" { interpolationDepth += 1 }
                        if source[index] == ")" { interpolationDepth -= 1 }
                        index = source.index(after: index)
                    }
                    continue
                }
                if character == "\\" {
                    index = next < source.endIndex ? source.index(after: next) : source.endIndex
                    continue
                }
                if character == "\"" {
                    literals.append(current)
                    current = ""
                    inString = false
                } else {
                    current.append(character)
                }
            } else if character == "/", next < source.endIndex, source[next] == "/" {
                inLineComment = true
            } else if character == "\"" {
                inString = true
            }
            index = next
        }
        return literals
    }
}

final class RoutingBindingTests: XCTestCase {
    // MARK: - The declaration

    /// The port and folder boxes are the override, and an override that can
    /// be typed into while the switch is off is an invitation to declare a
    /// proxy nobody turned on.
    ///
    /// Asserted on the argument rather than on the presence of `.disabled`:
    /// `.disabled(form.on)` is the inversion that would ship the card with
    /// exactly the wrong two fields live.
    func testThePortAndFolderFieldsAreLiveOnlyWhileTheSwitchIsOn() throws {
        let body = try XCTUnwrap(RoutingCard.body())

        // The port is still a box; the folder is a chooser, because on
        // this platform pointing at a directory through the system's own
        // panel is what a person can answer and a path string is not.
        for (label, control) in [
            ("copy.portTitle", "TextField("),
            ("copy.folderTitle", "Button(copy.chooseFolder)"),
        ] {
            let group = try XCTUnwrap(
                RoutingCard.region(of: body, from: "TCFieldLabel(\(label))", to: ".disabled(")
            )
            XCTAssertTrue(
                group.contains(control),
                "the group gated after \(label) holds no \(control): \(group)"
            )
            let argument = try XCTUnwrap(
                RoutingCard.disabledArgument(after: "TCFieldLabel(\(label))", in: body)
            )
            XCTAssertEqual(
                argument, "!form.on",
                "the \(label) group is gated on `\(argument)`, not on the switch being on"
            )
        }

        // And the chooser is a chooser: directories only, nothing created,
        // one answer. Every other affordance on that panel is a way to give
        // an answer that cannot be right.
        let panel = try XCTUnwrap(
            RoutingCard.declaration("private func chooseIronWireFolder() -> String? {")
        )
        XCTAssertTrue(panel.contains("panel.canChooseDirectories = true"), panel)
        XCTAssertTrue(panel.contains("panel.canChooseFiles = false"), panel)
        XCTAssertTrue(panel.contains("panel.allowsMultipleSelection = false"), panel)
        XCTAssertTrue(panel.contains("panel.canCreateDirectories = false"), panel)

        let applyArgument = try XCTUnwrap(
            RoutingCard.region(of: body, from: "buttonStyle(.bordered)\n", to: "\n")
        )
        XCTAssertEqual(
            applyArgument.trimmingCharacters(in: .whitespaces),
            ".disabled(!form.on || model.routingChecking)",
            "the Apply button is gated on `\(applyArgument)`"
        )
    }

    /// A displayed default must never become a declaration.
    ///
    /// The port field shows IronWire's conventional number so nobody has to
    /// know it. Typing in it, or leaving it alone, writes nothing: the only
    /// two things on this card that reach `set_settings` are the switch and
    /// the Apply button, and a third writer hiding in a field's setter would
    /// have this window announce a local service nobody mentioned.
    func testOnlyTheSwitchAndTheTwoButtonsWriteTheDeclaration() throws {
        let body = try XCTUnwrap(RoutingCard.body())

        // Three, and each one is a thing a contributor pressed: the switch,
        // Apply, and the connect button discovery offers. The count is the
        // point -- a fourth writer is a writer that is not a press, and the
        // only place one can hide on this card is a field's setter.
        XCTAssertEqual(
            RoutingCard.occurrences(of: "model.applyIronWire(", in: body), 3,
            "the routing card has a writer besides the switch, Apply and connect"
        )

        // The one discovery added. It writes the form that is ON SCREEN,
        // turned on -- not one rebuilt from the discovered port -- so a
        // press cannot declare a number different from the one displayed.
        let connect = try XCTUnwrap(
            RoutingCard.region(of: body, from: "Button(copy.connect) {", to: "buttonStyle(")
        )
        XCTAssertTrue(
            connect.contains("RoutingSurface.connecting(form)"),
            "the connect button builds its own form: \(connect)"
        )
        XCTAssertTrue(connect.contains("model.applyIronWire(next)"), connect)
        XCTAssertFalse(
            connect.contains("model.routingDiscovery.port"),
            "the connect button declares the discovered port rather than the shown one"
        )

        // Discovery itself writes nothing. It is offered above the
        // disclosure and reads a file; a declaration on that path would be
        // this window announcing a local service nobody mentioned.
        let offer = try XCTUnwrap(
            RoutingCard.region(
                of: body, from: "RoutingSurface.discoveryLine(", to: "Button(copy.connect)"
            )
        )
        XCTAssertFalse(
            offer.contains("applyIronWire"),
            "showing what was discovered declares it: \(offer)"
        )
        let lookAgain = try XCTUnwrap(
            RoutingCard.region(of: body, from: "Button(copy.lookAgain)", to: "\n")
        )
        XCTAssertTrue(lookAgain.contains("model.discoverRouting()"), lookAgain)
        XCTAssertFalse(lookAgain.contains("applyIronWire"), lookAgain)

        for label in ["copy.portTitle", "copy.folderTitle"] {
            let group = try XCTUnwrap(
                RoutingCard.region(of: body, from: "TCFieldLabel(\(label))", to: ".disabled(")
            )
            XCTAssertFalse(
                group.contains("applyIronWire"),
                "the \(label) field writes the declaration as it is typed in"
            )
            XCTAssertTrue(
                group.contains("routingDraft = next"),
                "the \(label) field does not hold its edit in the draft: \(group)"
            )
        }

        // The conventional port is a value this card shows, never one it
        // sends on its own -- and it is the surface's constant, not a number
        // spelled again here.
        XCTAssertFalse(
            body.contains("\(RoutingForm.conventionalPort)"),
            "the conventional port is written into the view rather than read from RoutingForm"
        )
    }

    /// Turning it off hands the model an off form and nothing else.
    ///
    /// `RoutingCallTests.testTurningItOffWritesNullAndNotAnAbsentKey` proves
    /// what an off form becomes on the wire. This is the half above it: that
    /// the switch produces an off form at all, carrying whatever port was on
    /// screen rather than stamping one into it on the way past.
    func testTheSwitchWritesOnlyItsOwnFieldAndThatSpellsNull() throws {
        let body = try XCTUnwrap(RoutingCard.body())
        let setter = try XCTUnwrap(
            RoutingCard.region(of: body, from: "Toggle(copy.toggle, isOn: Binding(", to: "))")
        )

        XCTAssertTrue(setter.contains("get: { form.on }"), "the switch does not read the form: \(setter)")
        XCTAssertEqual(
            RoutingCard.occurrences(of: "next.", in: setter), 1,
            "the switch mutates something besides `on`: \(setter)"
        )
        XCTAssertTrue(setter.contains("next.on = on"), setter)
        XCTAssertTrue(setter.contains("model.applyIronWire(next)"), setter)

        // And the form that reaches is the one that spells off as null.
        let off = RoutingSurface.settingsParams(
            RoutingForm(on: false, port: 9001, tokenDir: "/Users/x/ironwire")
        )
        XCTAssertTrue(off["ironwire"] is NSNull, "off did not spell null: \(off)")
    }

    // MARK: - What the machine already knows

    /// The card asks before it asks the contributor.
    ///
    /// `discover_routing` has been in the daemon since it was written and
    /// no shell had ever called it, which is the whole reason this screen
    /// asked for a port at all. So this pins the call site: the card asks
    /// on appear, and it asks the model rather than reading a file itself.
    func testTheCardAsksWhatTheMachineKnowsWhenItAppears() throws {
        let body = try XCTUnwrap(RoutingCard.body())
        let appear = try XCTUnwrap(RoutingCard.region(of: body, from: ".onAppear {", to: "}"))
        XCTAssertTrue(
            appear.contains("model.discoverRouting()"),
            "the card no longer asks what the machine knows: \(appear)"
        )
    }

    /// The offer is a sentence the Rust assembled, rendered unchanged.
    ///
    /// Both branches of it -- a port was published, or nothing was -- and
    /// neither is an error state here. A machine without IronWire is the
    /// ordinary machine, and there is no `else` on this card for it to fall
    /// into.
    func testTheDiscoveryOfferIsTheSharedSentence() throws {
        let body = try XCTUnwrap(RoutingCard.body())
        XCTAssertTrue(
            body.contains("RoutingSurface.discoveryLine("),
            "the discovery offer is not built from the surface: \(body)"
        )
        XCTAssertTrue(
            body.contains("model.routingDiscovery, copy: copy, calls: model.routingCalls"),
            "the offer is not built from what was discovered and the shared sentences"
        )
    }

    /// The contributor's declared port always wins.
    ///
    /// A pointer is a file that survives the daemon that wrote it --
    /// IronWire removes it only on a clean stop -- so a stale one naming
    /// 9000 must not replace a declared 8463 in the field. This is the same
    /// rule `ironwire_ledger_for` obeys on the reading side.
    func testADiscoveredPortNeverReplacesADeclaredOne() {
        let declared = RoutingForm.fromDeclaration(
            mode: "watch", port: 8463, tokenDir: nil, discoveredPort: 9000
        )
        XCTAssertEqual(declared.port, 8463)

        // And where nothing is declared it fills in, ahead of the
        // conventional number -- which is a display, not a declaration.
        let undeclared = RoutingForm.fromDeclaration(
            mode: nil, port: nil, tokenDir: nil, discoveredPort: 9000
        )
        XCTAssertEqual(undeclared.port, 9000)
        XCTAssertFalse(undeclared.on)
        XCTAssertTrue(RoutingSurface.settingsParams(undeclared)["ironwire"] is NSNull)

        let nothingAnywhere = RoutingForm.fromDeclaration(
            mode: nil, port: nil, tokenDir: nil, discoveredPort: nil
        )
        XCTAssertEqual(nothingAnywhere.port, RoutingForm.conventionalPort)
    }

    /// The fields become a disclosure only where the machine answered.
    ///
    /// Where it did not they are the only way to answer, so they stay open.
    /// This inverts the default; it does not remove the manual path.
    func testThePortAndFolderCollapseOnlyOnceSomethingWasDiscovered() throws {
        XCTAssertTrue(
            RoutingSurface.overrideIsCollapsed(RoutingDiscovery(port: 9143, tokenPath: nil))
        )
        XCTAssertFalse(RoutingSurface.overrideIsCollapsed(.none))

        let body = try XCTUnwrap(RoutingCard.body())
        let disclosure = try XCTUnwrap(
            RoutingCard.region(of: body, from: "DisclosureGroup(", to: "TCFieldLabel(copy.portTitle)")
        )
        XCTAssertTrue(disclosure.contains("copy.overrideTitle"), disclosure)
        XCTAssertTrue(
            disclosure.contains("RoutingSurface.overrideIsCollapsed(model.routingDiscovery)"),
            "the disclosure's default is not what was discovered: \(disclosure)"
        )
        XCTAssertTrue(
            disclosure.contains("routingOverrideOpen"),
            "a contributor who opens the disclosure is not obeyed: \(disclosure)"
        )
    }

    // MARK: - Per-tool words

    /// The words come from `probe_routed_tools`, never from the switch.
    ///
    /// This is the defect the whole surface replaced: the declaration used
    /// to be the only input to a tool's word, which let a contributor read
    /// the wired word on the same card as "nothing answered". A row that
    /// reaches for `form` at all has reintroduced it.
    func testThePerToolWordsComeFromTheProbeAndNeverFromTheSwitch() throws {
        let body = try XCTUnwrap(RoutingCard.body())
        let rows = try XCTUnwrap(
            RoutingCard.region(of: body, from: "ForEach(", to: "accessibilityLabel(")
        )

        XCTAssertTrue(
            rows.contains("evidence: model.routingEvidence"),
            "the rows are not built from what IronWire answered: \(rows)"
        )
        XCTAssertTrue(
            rows.contains("sourceModes: model.daemonSettings?.routingSourceModes ?? .unset"),
            "the rows are not built from the daemon's source modes: \(rows)"
        )
        for banned in ["form.on", "form.port", "form.tokenDir", "routingDraft", "routingChecking"] {
            XCTAssertFalse(
                rows.contains(banned),
                "a tool row reads `\(banned)`, which is this app's declaration and not IronWire's answer"
            )
        }
    }

    /// No verdict is derived from the rendered word.
    ///
    /// The row's tone now arrives **on the row**, decided by the same shared
    /// branch table that chose the word, from the same two inputs. It used
    /// to be recovered here by comparing the rendered word against the
    /// payload's private field -- which was already a text comparison
    /// against a privacy claim, one `contains` away from the bug that
    /// matched "unreachable" as "reachable" on this same surface, and
    /// `Private` is a substring of the denial that must never come back.
    func testNoStylingDecisionIsMadeAgainstARenderedString() throws {
        let body = try XCTUnwrap(RoutingCard.body())
        let rows = try XCTUnwrap(
            RoutingCard.region(of: body, from: "ForEach(", to: "accessibilityLabel(")
        )
        XCTAssertTrue(
            rows.contains("tone(row.tone)"),
            "the row's tone is not the one the shared table put on the row: \(rows)"
        )
        // And it is not recovered from the word on the way past.
        for recovered in ["forWord:", "copy.wordPrivate", "wordPrivate"] {
            XCTAssertFalse(
                rows.contains(recovered),
                "a tone decision reads the rendered word: \(recovered)"
            )
        }
        for banned in ["row.word ==", "== row.word", "row.word.contains", "\"Private\""] {
            XCTAssertFalse(rows.contains(banned), "a tone decision reads the rendered word: \(banned)")
        }
    }

    // MARK: - The status line

    /// One state drives the sentence and the stamp.
    ///
    /// "Last checked" is a stamp on the running daemon -- never an install
    /// date, never a connected-since -- and it is only shown on a state that
    /// has actually had an answer. A stamp gated on a different state than
    /// the sentence it sits under is a card claiming a check that never
    /// happened.
    func testTheSentenceAndTheStampReadTheSameDaemonState() throws {
        let body = try XCTUnwrap(RoutingCard.stateBody())

        XCTAssertTrue(
            body.contains("let state = model.status.routing.state"),
            "the status line no longer reads the daemon's state: \(body)"
        )
        XCTAssertTrue(
            body.contains("RoutingSurface.stateLine(state, copy: copy, calls: model.routingCalls)"),
            "the sentence is not built from that state: \(body)"
        )
        XCTAssertTrue(
            body.contains(
                "RoutingSurface.showsLastChecked(forState: state, calls: model.routingCalls)"
            ),
            "the stamp is not gated on that same state: \(body)"
        )
        XCTAssertTrue(
            body.contains("model.status.routing.lastRefreshAt"),
            "the stamp is not the daemon's per-process refresh time: \(body)"
        )
        XCTAssertTrue(
            body.contains("TCRoutingCopy.lastChecked("),
            "the stamp's sentence is not the shared one: \(body)"
        )

        // The gate is what keeps the stamp off a state that has had no
        // answer, so it must be a real state and not a constant.
        XCTAssertFalse(
            body.contains("showsLastChecked(forState: RoutingSurface.State."),
            "the stamp is gated on a fixed state rather than the daemon's"
        )
        XCTAssertFalse(body.contains("Date.distantPast"), body)
    }

    /// The status line is painted, and from the daemon's state rather than
    /// from the sentence that state produced.
    ///
    /// `tone(forState:)` was public, documented as the thing that keeps
    /// `awaiting_rows` from reading as a fault, and reached from this view
    /// only through `showsLastChecked` -- so it gated the stamp and nothing
    /// ever painted with it. GTK has painted this row from the same three
    /// states since it was written; this is that parity, asserted.
    func testTheStatusSentenceIsPaintedFromTheStateAndNotFromItsOwnText() throws {
        let body = try XCTUnwrap(RoutingCard.stateBody())

        XCTAssertTrue(
            body.contains(
                "let stateTone = tone("
                    + "RoutingSurface.tone(forState: state, calls: model.routingCalls))"
            ),
            "the status line's tone is not the surface's, from the daemon's state: \(body)"
        )
        XCTAssertTrue(
            body.contains("foregroundStyle(stateTone.textColor)"),
            "the status sentence is not painted with that tone: \(body)"
        )
        // Not recovered from the rendered sentence, the way the row's tone
        // once was from the rendered word.
        for recovered in [
            "stateLine(state, copy: copy, calls: model.routingCalls) ==",
            "copy.stateOff ==", "== copy.stateOff", "copy.stateReading ==", "copy.stateWaiting ==",
        ] {
            XCTAssertFalse(
                body.contains(recovered),
                "a tone decision reads the rendered sentence: \(recovered)"
            )
        }
    }

    /// `awaiting_rows` is not a fault.
    ///
    /// A contributor who has just changed anything on this card sees it
    /// until the daemon's next tick, because a reader built a moment ago
    /// starts empty by construction. Painting it as a fault accuses a
    /// working proxy of being broken at exactly that moment -- so no state
    /// on this card reaches for the two tones that mean something is wrong.
    func testNoStateOnThisCardIsPaintedAsAFault() throws {
        let card = try XCTUnwrap(RoutingCard.body())
        let state = try XCTUnwrap(RoutingCard.stateBody())
        let bridge = try XCTUnwrap(RoutingCard.toneBridge())

        XCTAssertEqual(RoutingSurface.tone(forState: "awaiting_rows", calls: routingCalls), .held)
        XCTAssertTrue(
            bridge.contains("case .held: return .held"),
            "the tone bridge no longer carries held through: \(bridge)"
        )
        for alarming in [".attention", ".refused", "TC.gold", "TC.red"] {
            XCTAssertFalse(
                card.contains(alarming),
                "the routing card paints something \(alarming)"
            )
            XCTAssertFalse(
                state.contains(alarming),
                "the routing status line paints something \(alarming)"
            )
            XCTAssertFalse(
                bridge.contains(alarming),
                "the routing tone bridge can produce \(alarming)"
            )
        }
    }

    // MARK: - The probe result

    /// The probe sentence is rendered exactly as the surface assembled it.
    ///
    /// All three outcomes -- and the token one naming the absolute path the
    /// daemon reported -- arrive on `model.routingProbeLine` already
    /// finished. Anything wrapped around it here is wording this shell
    /// invented about a proxy it did not check.
    func testTheProbeSentenceIsRenderedUnchanged() throws {
        let body = try XCTUnwrap(RoutingCard.body())
        XCTAssertTrue(
            body.contains("if let probeLine = model.routingProbeLine {"),
            "the probe sentence is no longer read from the model: \(body)"
        )
        XCTAssertTrue(body.contains("Text(probeLine)"), "the probe sentence is decorated rather than shown")
        XCTAssertEqual(
            RoutingCard.occurrences(of: "probeLine", in: body), 2,
            "the probe sentence is used somewhere besides its own line"
        )
    }

    // MARK: - Every string

    /// This card writes no wording of its own.
    ///
    /// Every visible string comes off `RoutingCopy` or arrives assembled
    /// through `TCRoutingCopy`. A literal here is a fourth place the
    /// vocabulary could drift, and -- since the words are what claim privacy
    /// -- the one place a stale claim would survive every copy test in the
    /// suite. The only literals allowed are punctuation and the empty
    /// placeholders SwiftUI's `TextField` requires.
    func testTheCardPrintsNoWordsOfItsOwn() throws {
        for source in [try XCTUnwrap(RoutingCard.body()), try XCTUnwrap(RoutingCard.stateBody())] {
            for literal in RoutingCard.stringLiterals(in: source) {
                XCTAssertFalse(
                    literal.contains(where: \.isLetter),
                    "the routing card carries wording of its own: \"\(literal)\""
                )
            }
        }
    }

    /// Nothing on this card sends anybody to start anything again.
    ///
    /// The daemon applies a changed declaration to itself; the card says so
    /// out loud, in the Rust's words. A restart notice added here would be
    /// describing a product this is not, and would be the one sentence on
    /// the card that no copy test could see.
    func testTheCardCarriesNoRestartNotice() throws {
        let card = try XCTUnwrap(RoutingCard.body())
        XCTAssertTrue(card.contains("copy.appliesAtOnce"), "the card no longer says it applies at once")
        for banned in ["restart", "relaunch", "reopen", "reboot", "quit"] {
            XCTAssertFalse(
                card.lowercased().contains(banned),
                "a restart notice reached the routing card: \(banned)"
            )
        }
    }

    /// The card renders nothing at all when the shared payload did not
    /// arrive, rather than falling back to wording of its own.
    func testTheCardRendersNothingWithoutTheSharedPayload() throws {
        let body = try XCTUnwrap(RoutingCard.body())
        XCTAssertTrue(
            body.trimmingCharacters(in: .whitespacesAndNewlines)
                .hasPrefix("if let copy = model.routingCopy {"),
            "the card is no longer guarded on the payload: \(body.prefix(200))"
        )
        XCTAssertFalse(body.contains("else {"), "the card has a fallback for a missing payload")
    }
}
