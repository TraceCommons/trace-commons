import TCBridge
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// How the glass witness section (`WitnessSection`) is wired to the surface
/// underneath it.
///
/// `WitnessSurfaceTests` proves the mapping and `WitnessExportTests` proves
/// the words. Neither can see the layer between them and a contributor:
/// which property each control is bound to, whether a refusal is painted in
/// the refusing tone, and whether a refusal is offered a way out. A card
/// that read the tone off the sentence, or that painted an unpinned witness
/// with the routing bridge, would pass both suites and ship the exact defect
/// this surface exists to prevent.
///
/// A SwiftUI `body` holding `@State` and an `@EnvironmentObject` cannot be
/// built, rendered or reflected outside a running window, so these assert
/// against the view's own source -- the same limitation, and the same
/// justification, as `RoutingBindingTests`. Every locator reports what it
/// was looking in when it fails, so a refactor that moves this card produces
/// a failure to fix rather than a test that quietly stops asserting.

private enum WitnessCard {
    /// `.../macos/Sources/TraceCommonsApp/Views/Settings/WitnessSection.swift`
    static let viewPath = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // TraceCommonsAppTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .appendingPathComponent("Sources/TraceCommonsApp/Views/Settings/WitnessSection.swift")

    static let modelPath = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp/AppModel.swift")

    /// The section's always-present container: the payload guard and the
    /// refresh on appear.
    static func container(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration("var body: some View {", in: viewPath, file: file, line: line)
    }

    /// The container and the card it draws, in that order. The glass
    /// section splits what the legacy `witness` property held across these
    /// two declarations.
    static func body(file: StaticString = #filePath, line: UInt = #line) -> String? {
        guard
            let container = container(file: file, line: line),
            let card = declaration(
                "private func card(_ copy: WitnessCopy) -> some View {",
                in: viewPath, file: file, line: line)
        else { return nil }
        return container + card
    }

    /// The inference-evidence and token-contribution cards, which the card
    /// draws below itself.
    static func evidenceBodies(file: StaticString = #filePath, line: UInt = #line) -> [String]? {
        guard
            let inference = declaration(
                "private func inferenceEvidence(_ copy: WitnessCopy) -> some View {",
                in: viewPath, file: file, line: line),
            let token = declaration(
                "private func tokenContribution(_ copy: WitnessCopy) -> some View {",
                in: viewPath, file: file, line: line)
        else { return nil }
        return [inference, token]
    }

    static func stateBody(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration(
            "private func stateBlock(_ code: Int32) -> some View {",
            in: viewPath, file: file, line: line)
    }

    static func fieldsBody(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration(
            "private func fields(_ copy: WitnessCopy) -> some View {",
            in: viewPath, file: file, line: line)
    }

    /// The `WitnessTone` -> `GlassStatus` bridge.
    static func toneBridge(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration(
            "static func tone(_ tone: WitnessTone) -> GlassStatus {",
            in: viewPath, file: file, line: line)
    }

    static func modelDeclaration(
        _ signature: String, file: StaticString = #filePath, line: UInt = #line
    ) -> String? {
        declaration(signature, in: modelPath, file: file, line: line)
    }

    /// The source between `signature` and the brace that closes it. Braces
    /// inside comments and string literals are not counted; this card
    /// carries both.
    static func declaration(
        _ signature: String, in path: URL,
        file: StaticString = #filePath, line: UInt = #line
    ) -> String? {
        guard let text = try? String(contentsOf: path, encoding: .utf8) else {
            XCTFail("could not read \(path.path)", file: file, line: line)
            return nil
        }
        guard let start = text.range(of: signature) else {
            XCTFail(
                "\(path.lastPathComponent) no longer declares `\(signature)`",
                file: file, line: line)
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

    /// Every string literal in `source`, holes removed and `//` comments
    /// skipped -- a comment is not something a contributor reads.
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

final class WitnessBindingTests: XCTestCase {

    // MARK: - The tone bridge

    /// A refusal is `.outside` and never `.ask`. Ask is caution rather than
    /// alarm -- the tone of a setup that is degraded but still working -- and
    /// a refusing witness is sending nothing at all.
    func testTheRefusedToneMapsToRefusedAndNeverToAttention() throws {
        let bridge = try XCTUnwrap(WitnessCard.toneBridge())
        XCTAssertTrue(
            bridge.contains("case .refused: return .outside"),
            "the refused tone no longer maps to GlassStatus.outside: \(bridge)")
        XCTAssertFalse(
            bridge.contains("case .refused: return .ask"),
            "a witness refusal is painted as caution: \(bridge)")
        XCTAssertFalse(
            bridge.contains("case .refused: return .off"),
            "a witness refusal is painted as nothing to say: \(bridge)")
        XCTAssertEqual(WitnessSection.tone(.refused), .outside)
        for calmer: WitnessTone in [.neutral, .held, .clear, .attention] {
            XCTAssertNotEqual(WitnessSection.tone(calmer), .outside, "\(calmer) reads as a refusal")
        }
    }

    /// Every case is spelled out. A `default` arm here is what would make a
    /// tone added later render as whatever the arm happened to be.
    func testTheToneBridgeHasNoDefaultArm() throws {
        let bridge = try XCTUnwrap(WitnessCard.toneBridge())
        XCTAssertFalse(bridge.contains("default"), "the witness tone bridge fell back: \(bridge)")
        // An arm may name two cases (`case .attention, .held:`), so each case
        // is looked for on a `case` line rather than as `case .x:` exactly.
        let arms = bridge.split(separator: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { $0.hasPrefix("case ") }
        for arm in ["neutral", "held", "clear", "attention", "refused"] {
            XCTAssertTrue(
                arms.contains { line in
                    line.prefix { $0 != ":" }.split(separator: ",")
                        .contains { $0.trimmingCharacters(in: .whitespaces)
                            .replacingOccurrences(of: "case ", with: "") == ".\(arm)" }
                },
                "no arm for \(arm)")
        }
    }

    /// The witness card must not paint itself through the routing bridge.
    /// The two ABI tone ranges are disjoint precisely so a cross-wired
    /// mapper is wrong for every value; two bridges is what keeps them from
    /// being wired together in the first place.
    func testTheCardDoesNotPaintItselfThroughTheRoutingBridge() throws {
        for source in [
            try XCTUnwrap(WitnessCard.body()),
            try XCTUnwrap(WitnessCard.stateBody()),
        ] {
            XCTAssertFalse(
                source.contains("RoutingTone"),
                "the witness card reaches for a routing tone: \(source)")
            XCTAssertFalse(
                source.contains("tone(RoutingSurface"),
                "the witness card reaches for the routing tone bridge: \(source)")
        }
    }

    // MARK: - The state

    /// The tone is taken from the state code, never recovered by comparing
    /// the rendered sentence against anything.
    func testTheToneIsTakenFromTheStateAndNotFromTheSentence() throws {
        let state = try XCTUnwrap(WitnessCard.stateBody())
        XCTAssertTrue(
            state.contains("WitnessSurface.tone(forState: code"),
            "the state's tone is no longer taken from the state: \(state)")
        for banned in [".contains(", "== line", "line =="] {
            XCTAssertFalse(
                state.contains(banned),
                "the witness card recovers something by comparing a sentence: \(banned)")
        }
    }

    /// A state this build cannot name produces no sentence, and the card
    /// renders none of its own: the sentence is rendered inside an `if let`
    /// over the ABI's answer, with no `else`.
    func testAnUnnameableStateRendersNoSentenceOfTheCardsOwn() throws {
        let state = try XCTUnwrap(WitnessCard.stateBody())
        XCTAssertTrue(
            state.contains("if let line = WitnessSurface.stateLine(code"),
            "the state sentence is no longer conditional on the ABI answering: \(state)")
        XCTAssertFalse(
            state.contains("} else {"),
            "the witness state has a fallback branch, which can only be wording: \(state)")
    }

    /// The tone still answers when the sentence does not. A card that
    /// derived the tone inside the `if let` would paint nothing at all in
    /// the one case that matters most.
    func testTheToneIsComputedOutsideTheSentencesConditional() throws {
        let state = try XCTUnwrap(WitnessCard.stateBody())
        let toneIndex = try XCTUnwrap(state.range(of: "let stateTone = Self.tone("))
        let lineIndex = try XCTUnwrap(state.range(of: "if let line = WitnessSurface.stateLine("))
        XCTAssertLessThan(
            toneIndex.lowerBound, lineIndex.lowerBound,
            "the tone is computed inside the sentence's branch: \(state)")
    }

    // MARK: - The way out of a refusal

    /// A refusal must have a way out. `AppModel.Startup.needsRoots` exists
    /// as a separate case from `.refused` because a refusal with no
    /// affordance rendered behind the daemon it was blocking, and a fresh
    /// install could never finish onboarding. A configured-but-unpinned
    /// witness must not become that trap.
    func testEveryRefusalIsOfferedAWayOut() throws {
        let card = try XCTUnwrap(WitnessCard.body())
        XCTAssertTrue(
            card.contains("WitnessSurface.offersClear(state)"),
            "the clear action is no longer gated on the shared rule: \(card)")
        XCTAssertTrue(
            card.contains("Button(copy.clear) { model.clearWitness() }"),
            "the clear action is gone: \(card)")
        // And the rule itself says yes to every refusal, including one this
        // build cannot name. Asserted here as well as in TCShellCoreTests
        // because it is the card's claim that is being made.
        for state: WitnessTrustState in [
            .refusingUnpinned, .refusingPinMalformed,
            .refusingInferenceReceiptsMissing, .unreadable, .unnameable(99),
        ] {
            XCTAssertTrue(WitnessSurface.offersClear(state), "\(state) has no way out")
        }
    }

    /// The other way out of the unpinned refusal is to pin a measurement,
    /// so the fields stay live in every refusing state.
    func testTheFieldsStayLiveInARefusal() throws {
        let card = try XCTUnwrap(WitnessCard.body())
        XCTAssertTrue(
            card.contains("WitnessSurface.offersConfigure(state)"),
            "the fields are no longer gated on the shared rule: \(card)")
        for state: WitnessTrustState in [
            .refusingUnpinned, .refusingPinMalformed, .unreadable, .unnameable(99),
        ] {
            XCTAssertTrue(WitnessSurface.offersConfigure(state), "\(state)")
        }
    }

    // MARK: - The controls

    /// Each field writes its own property. A card whose signing-key box wrote
    /// the URL would pass every other suite in the tree.
    func testEachFieldIsBoundToItsOwnProperty() throws {
        let fields = try XCTUnwrap(WitnessCard.fieldsBody())
        for (label, property) in [
            ("GlassTextField(copy.urlTitle", "next.url = value"),
            ("GlassTextField(copy.signingAddressTitle", "next.signingAddress = value"),
            ("Text(copy.measurementsTitle)", "next.measurements = value"),
        ] {
            let anchor = try XCTUnwrap(
                fields.range(of: label), "no field labelled \(label)")
            let rest = String(fields[anchor.upperBound...])
            let setter = try XCTUnwrap(
                rest.range(of: "next."), "nothing after \(label) writes anything")
            XCTAssertTrue(
                rest[setter.lowerBound...].hasPrefix(property),
                "the field labelled \(label) writes the wrong property")
        }
    }

    /// This shell does not offer the button that would write an unpinned
    /// witness. Doing so produces a client that refuses every submission
    /// from the moment it is saved: a total upload outage, entered by
    /// pressing a button that looked like configuration.
    func testConfigureIsRefusedUntilSomethingIsPinned() throws {
        let fields = try XCTUnwrap(WitnessCard.fieldsBody())
        XCTAssertTrue(
            fields.contains(".disabled(!form.canConfigure || model.witnessBusy)"),
            "the configure button is no longer gated on a pinned measurement: \(fields)")
        XCTAssertFalse(
            WitnessForm(url: "https://w.example", signingAddress: "0xabc", measurements: "")
                .canConfigure)
    }

    /// Before the daemon answers, neither evidence switch can be pressed: an
    /// absent setting is neither on nor off, so Enable and Disable both wait
    /// for it, and the status line is the loading state rather than a word.
    func testTheEvidenceSwitchesWaitForTheSettingTheyChange() throws {
        let bodies = try XCTUnwrap(WitnessCard.evidenceBodies())
        for (body, enable, disable, setting) in [
            (bodies[0], "Button(copy.inferenceEnable)", "Button(copy.inferenceDisable)",
             "model.inferenceEvidenceBusy || model.daemonSettings?.ironwireAttestedBodies == nil"),
            (bodies[1], "Button(copy.tokenEnable ?? \"\")", "Button(copy.tokenDisable ?? \"\")",
             "model.tokenContributionBusy || model.daemonSettings?.tokenDistributionsContribution == nil"),
        ] {
            for control in [enable, disable] {
                let anchor = try XCTUnwrap(body.range(of: control), "no \(control)")
                let call = try XCTUnwrap(
                    body.range(of: ".disabled(", range: anchor.upperBound..<body.endIndex), "\(control) is never gated")
                let line = body[call.upperBound...].prefix { $0 != "\n" }
                XCTAssertEqual(String(line), setting + ")", "\(control) is gated on `\(line)`")
            }
            XCTAssertTrue(
                body.contains("if model.daemonSettings == nil {\n                    SettingsAwaiting()\n"),
                "the status line is drawn before the daemon answers")
        }
    }

    // MARK: - The pinned measurements

    /// The count is a sentence from the Rust, never a bare numeral. A number
    /// with no words around it on a privacy surface is this shell authoring
    /// wording by omission.
    func testTheCountIsRenderedAsASentenceAndNeverAsABareNumeral() throws {
        let fields = try XCTUnwrap(WitnessCard.fieldsBody())
        XCTAssertTrue(
            fields.contains("model.witnessStatus?.pinnedMeasurementLine"),
            "the card no longer renders the shared count sentence: \(fields)")
        XCTAssertFalse(
            fields.contains("pinnedMeasurementCount"),
            "the card renders a bare count: \(fields)")
    }

    /// The count sentence is null for `absent`, `not_enrolled` and
    /// `unreadable`, and the card then renders NOTHING -- a count of the
    /// pins on a witness that does not exist is a wrong sentence, not a
    /// short one, so there is no `else` and no placeholder.
    func testANullCountLineRendersNothingRatherThanAPlaceholder() throws {
        let fields = try XCTUnwrap(WitnessCard.fieldsBody())
        let region = try XCTUnwrap(
            fields.range(of: "if let line = model.witnessStatus?.pinnedMeasurementLine"),
            "the count sentence is no longer conditional on the ABI answering")
        let after = String(fields[region.upperBound...])
        // The next thing after the conditional's own block must not be an
        // `else`. Checked over the following few lines rather than the whole
        // body, so an unrelated `else` later on cannot mask this.
        let window = String(after.prefix(400))
        XCTAssertFalse(
            window.contains("} else {"),
            "the count sentence has a fallback, which can only be wording: \(window)")
    }

    /// The editor is pre-filled from what the ABI returned, so changing a
    /// URL does not mean retyping every pin from memory.
    func testTheMeasurementsEditorIsPreFilledFromWhatWasRead() throws {
        let fields = try XCTUnwrap(WitnessCard.fieldsBody())
        XCTAssertTrue(
            fields.contains("WitnessForm.fromStatus(model.witnessStatus)"),
            "the fields are no longer seeded from the status: \(fields)")
        // And the seeding reads the entries rather than inventing lines.
        let seeded = WitnessForm.fromStatus(
            WitnessStatus(
                stateCode: 1, refusal: nil, url: "https://w.example",
                signingAddress: "0xabc", pinnedMeasurementCount: 2,
                pinnedMeasurementLine: "2 measurements are pinned.",
                pinnedMeasurements: ["mrtd=aa", "mrtd=bb"]))
        XCTAssertEqual(seeded.measurements, "mrtd=aa\nmrtd=bb")
    }

    /// Nothing between the read and the write rewrites an entry. The editor
    /// hands back exactly what it was given.
    func testTheEditorDoesNotRewriteAPin() throws {
        let stored = ["  mrtd=aa,mrconfigid=bb  ", "mrtd=nothexatall"]
        let form = WitnessForm.fromStatus(
            WitnessStatus(
                stateCode: 3, refusal: "witness_expected_measurement_malformed",
                url: "https://w.example", signingAddress: "0xabc",
                pinnedMeasurementCount: stored.count,
                pinnedMeasurementLine: "2 measurements are pinned.",
                pinnedMeasurements: stored))
        XCTAssertEqual(form.measurementLines, stored)
        let json = try XCTUnwrap(form.measurementsJSON)
        let decoded = try JSONDecoder().decode(
            [String].self, from: XCTUnwrap(json.data(using: .utf8)))
        XCTAssertEqual(decoded, stored)
    }

    // MARK: - The words

    /// Not one word on this card is written here. Several of these
    /// sentences are privacy claims, and a hand-written copy of one stops
    /// matching the day the claim changes with nothing to notice.
    func testNoWordingIsAuthoredOnThisCard() throws {
        for source in [
            try XCTUnwrap(WitnessCard.body()),
            try XCTUnwrap(WitnessCard.stateBody()),
            try XCTUnwrap(WitnessCard.fieldsBody()),
        ] + (try XCTUnwrap(WitnessCard.evidenceBodies())) {
            for literal in WitnessCard.stringLiterals(in: source) {
                XCTAssertFalse(
                    literal.contains(where: \.isLetter),
                    "the witness card carries wording of its own: \"\(literal)\"")
            }
        }
    }

    /// A certificate records what was removed and the risk that was judged
    /// left. It is not a statement that a session is clean, and no shell may
    /// summarise it as one.
    func testTheCardNeverClaimsATraceIsCleanOrAttested() throws {
        let sources = [
            try XCTUnwrap(WitnessCard.body()),
            try XCTUnwrap(WitnessCard.stateBody()),
            try XCTUnwrap(WitnessCard.fieldsBody()),
        ] + (try XCTUnwrap(WitnessCard.evidenceBodies()))
        for source in sources {
            for literal in WitnessCard.stringLiterals(in: source) {
                let lowered = literal.lowercased()
                for banned in ["attested", "genuine", "verified", "clean", "safe"] {
                    XCTAssertFalse(lowered.contains(banned), "\"\(literal)\"")
                }
            }
        }
    }

    /// The card renders nothing at all when the shared payload did not
    /// arrive, rather than falling back to wording of its own.
    ///
    /// The glass container keeps an `else` so the refresh on appear hangs on
    /// something present either way; that branch draws an empty, hidden
    /// frame and no words.
    func testTheCardRendersNothingWithoutTheSharedPayload() throws {
        let container = try XCTUnwrap(WitnessCard.container())
        XCTAssertTrue(
            container.contains("if let copy = model.witnessCopy {\n                card(copy)"),
            "the card is no longer conditional on the shared payload")
        guard
            let start = container.range(of: "} else {"),
            let end = container.range(of: ".onAppear {", range: start.upperBound..<container.endIndex)
        else {
            return XCTFail("the container no longer has its unavailable branch: \(container)")
        }
        let fallback = String(container[start.upperBound..<end.lowerBound])
        XCTAssertTrue(
            fallback.contains("Color.clear.frame(width: 0, height: 0).accessibilityHidden(true)"),
            "the missing-payload branch draws something: \(fallback)")
        for drawn in ["Text(", "Label(", "Button(", "Glass"] {
            XCTAssertFalse(
                fallback.contains(drawn),
                "the card has a fallback branch, which can only be wording of its own: \(drawn)")
        }
    }

    /// Nothing on this card sends anybody to start anything again: a changed
    /// witness is read on the next upload.
    func testTheCardCarriesNoRestartNotice() throws {
        let card = try XCTUnwrap(WitnessCard.body())
        XCTAssertTrue(card.contains("copy.appliesAtOnce"))
        for banned in ["restart", "relaunch", "reboot", "quit"] {
            XCTAssertFalse(
                card.lowercased().contains(banned), "a restart notice reached the card: \(banned)")
        }
    }

    // MARK: - Nothing is applied optimistically

    /// The card publishes what came back, never what was asked for. A write
    /// that set the state from its own argument would show a pinned witness
    /// the moment the button was pressed, whatever the file says.
    func testAWriteRepublishesWhatWasReadBack() throws {
        let write = try XCTUnwrap(
            WitnessCard.modelDeclaration(
                "private func writeWitness(_ work: @escaping @Sendable (String) -> TCWitness.Outcome) {"
            ))
        XCTAssertTrue(
            write.contains("TCWitness.trustState(configDir: dir)"),
            "the state is not re-read after a write: \(write)")
        XCTAssertTrue(
            write.contains("TCWitness.statusJSON(configDir: dir)"),
            "the status is not re-read after a write: \(write)")
        // The write's own answer decides only whether a fixed label is
        // shown. It must not decide the state.
        XCTAssertFalse(
            write.contains("witnessStateCode = "),
            "a write assigns the state directly: \(write)")
    }

    /// The state comes from `tc_witness_trust_state` and never from the
    /// status having a URL in it. That is the boolean this surface refuses
    /// to hand a shell, spelled differently.
    func testTheStateIsNeverDerivedFromTheConfigurationHavingAUrl() throws {
        let model = try XCTUnwrap(
            WitnessCard.modelDeclaration("var witnessState: WitnessTrustState? {"))
        XCTAssertTrue(model.contains("witnessStateCode.map(WitnessTrustState.fromABI)"), model)
        XCTAssertFalse(model.contains("url"), "the state is derived from the URL: \(model)")

        let card = try XCTUnwrap(WitnessCard.body())
        XCTAssertFalse(
            card.contains("witnessStatus?.url != nil"),
            "the card derives a state from the URL being present: \(card)")
    }
}
