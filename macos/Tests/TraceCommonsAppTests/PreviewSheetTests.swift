import XCTest
import TCBridge
@testable import TCShellCore
@testable import TraceCommonsApp

/// Look inside, read-only (#1241 Task 7, Ron's `PreviewInspector`).
///
/// The card in the inspector is the review and carries the one approve
/// control; this sheet only shows what would be sent. A SwiftUI `body`
/// holding `@State` cannot be built or reflected outside a running window,
/// so the view's wiring is asserted against its own source, as
/// `AdmissionPlacementTests` does; the arithmetic behind the turn index,
/// the paging and the witness offer is asserted directly on `LookInside`.
final class PreviewSheetTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    static func sheet() throws -> String { try text("Views/PreviewSheet.swift") }

    /// The core's Look-inside words, decoded from the real export.
    static let words = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())?.lookInside

    /// The source between `signature` and the brace that closes it, with
    /// braces inside comments and string literals not counted.
    static func declaration(_ signature: String, in text: String) throws -> String {
        let start = try XCTUnwrap(text.range(of: signature), "PreviewSheet.swift no longer declares `\(signature)`")
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
        XCTFail("the body of `\(signature)` is unterminated")
        return ""
    }

    /// The source with `//` and `///` comment lines removed: a comment that
    /// explains why Contribute left is not a Contribute.
    static func code(_ text: String) -> String {
        text.split(separator: "\n", omittingEmptySubsequences: false)
            .filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
            .joined(separator: "\n")
    }

    // MARK: - Read-only

    /// One approve control, and it is the card's: nothing on this sheet
    /// approves, answers the verdict, corrects, or dismisses the session.
    /// The footer is Close and nothing else, and Escape closes it.
    func test_lookInsideHasNoApproveControl() throws {
        let sheet = Self.code(try Self.sheet())
        for needle in [
            "model.approve(", "contribute()", "Button(\"Contribute\")", "canContribute", "VerdictCopy.",
            "CorrectionCopy.", "ContributorVerdict", "correction", "model.dismiss(", "\"Not this one\"",
            "ScrubbingCaveatAtCommit()", "EligibilitySurface.mayProceed(",
        ] {
            XCTAssertFalse(sheet.contains(needle), "Look inside still carries \(needle)")
        }
        let footer = try Self.declaration("private var footer: some View {", in: sheet)
        XCTAssertEqual(footer.components(separatedBy: "Button(").count - 1, 1, "the footer holds one control: \(footer)")
        XCTAssertTrue(footer.contains("Button(close) { dismiss() }"), "the one control is the core's Close")
        // Never a Close with no name: the Look-inside table's word, else the
        // core's other Close, else no visible control (Escape still closes).
        XCTAssertFalse(footer.contains("?? \"\""), "a missing table must not draw an unnamed control")
        XCTAssertTrue(footer.contains("if let close = closeWord {"))
        XCTAssertTrue(sheet.contains("private var closeWord: String? { words?.close ?? Self.fallbackClose }"))
        // With no Close word, Escape still closes, from an unseen control.
        XCTAssertTrue(sheet.contains("if closeWord == nil {\n                Button(\"\") { dismiss() }\n"
                                     + "                    .keyboardShortcut(.cancelAction)\n"))
        XCTAssertTrue(sheet.contains("TCCoreCopy.firstRunCopyJSON().flatMap(FirstRunCopy.decode)?.passkey.close"),
                      "the fallback Close is the core's word too")
        XCTAssertTrue(footer.contains(".keyboardShortcut(.cancelAction)"), "Escape closes the sheet")
        XCTAssertFalse(footer.contains(".keyboardShortcut(.defaultAction)"))
        XCTAssertFalse(Self.words?.close.isEmpty ?? true, "the core names Close")
    }

    // MARK: - Witness review, up front

    /// Offered on every preview of a session that supports it -- not only
    /// after a preview failed -- in Ron's native-review block above the
    /// tabs, through Ron's consent overlay with its confirmation tick.
    func test_witnessReviewIsOfferedUpFront() throws {
        XCTAssertTrue(LookInside.offersWitnessReview(supported: true, witnessPinned: true, holdsCertificate: false))
        XCTAssertFalse(LookInside.offersWitnessReview(supported: false, witnessPinned: true, holdsCertificate: false))
        XCTAssertFalse(LookInside.offersWitnessReview(supported: true, witnessPinned: false, holdsCertificate: false))
        XCTAssertFalse(LookInside.offersWitnessReview(supported: true, witnessPinned: true, holdsCertificate: true),
                       "a session that already holds a certificate is not reviewed again")
        XCTAssertTrue(LookInside.showsNativeReview(admissionOffered: false, offersWitness: true, holdsCertificate: false))
        XCTAssertTrue(LookInside.showsNativeReview(admissionOffered: true, offersWitness: false, holdsCertificate: false))
        XCTAssertFalse(LookInside.showsNativeReview(admissionOffered: false, offersWitness: false, holdsCertificate: false))
        XCTAssertFalse(LookInside.showsNativeReview(admissionOffered: true, offersWitness: true, holdsCertificate: true))

        let sheet = Self.code(try Self.sheet())
        let header = try Self.declaration("private var header: some View {", in: sheet)
        XCTAssertTrue(header.contains("nativeReview"), "the native review sits in the header, above the tabs")
        let review = try Self.declaration("private var nativeReview: some View {", in: sheet)
        for needle in [
            "LookInside.offersWitnessReview(", "witnessSupported", "words.requestWitnessReview",
            // The witness must be pinned, and the certificate is read from
            // the live row, never only the copy the sheet opened with.
            "witnessPinned: model.witnessStateCode == 1", "holdsCertificate: holdsCertificate",
            "confirmingWitness = true", "model.daemonSettings?.admissionEvidenceOffered == true",
            "AdmissionPreparationView(entryID:", "words.nativeReview", "words.nativeReviewCaption",
        ] {
            XCTAssertTrue(review.contains(needle), "the native review lacks \(needle)")
        }
        let content = try Self.declaration("private var content: some View {", in: sheet)
        XCTAssertFalse(content.contains("confirmingWitness = true"), "the review is no longer reached by failing first")
        XCTAssertFalse(content.contains("AdmissionPreparationView"))
        // Ron's consent overlay: the request is sent only after the tick.
        XCTAssertTrue(sheet.contains("WitnessReviewConsent(copy: copy, confirmLine: words?.witnessConfirmLine,"))
        let consentStart = try XCTUnwrap(sheet.range(of: "struct WitnessReviewConsent: View {")).lowerBound
        let consent = String(sheet[consentStart...])
        XCTAssertTrue(consent.contains(".toggleStyle(GlassCheckboxStyle())"))
        // Fail closed: a caller without the core's tick line cannot confirm.
        XCTAssertTrue(consent.contains(".disabled(confirmLine == nil || !confirmed)"))
        XCTAssertFalse(consent.contains(".disabled(confirmLine != nil && !confirmed)"))
        // Every caller passes the core's tick line, the screenshot hook included.
        let screenshot = try Self.text("DebugScreenshot.swift")
        XCTAssertFalse(screenshot.contains("WitnessReviewConsent(copy: copy, onConfirm: {})"),
                       "the screenshot draws the overlay without Ron's tick")
        XCTAssertTrue(screenshot.contains("confirmLine: "))
    }

    /// A review that succeeds reopens the sheet on a body that now holds a
    /// certificate. The sheet's `entry` is the copy it opened with, so the
    /// certificate is read from the live row and from the reopened summary:
    /// Request witness review is not offered again for that session.
    func test_aCertificateEarnedWhileOpenEndsTheWitnessOffer() throws {
        XCTAssertFalse(LookInside.holdsCertificate(liveRow: false, envelopeDigest: nil))
        XCTAssertFalse(LookInside.holdsCertificate(liveRow: false, envelopeDigest: "sha256:ab"))
        XCTAssertTrue(LookInside.holdsCertificate(liveRow: true, envelopeDigest: nil))
        XCTAssertTrue(LookInside.holdsCertificate(liveRow: false, envelopeDigest: "witness-sha256:ab"),
                      "a body reopened under a witness digest holds a certificate")

        let sheet = Self.code(try Self.sheet())
        let held = try Self.declaration("private var holdsCertificate: Bool {", in: sheet)
        XCTAssertTrue(held.contains("model.awaitingDecision.first(where: { $0.entryID == entry.entryID })"),
                      "the certificate is read from the live row: \(held)")
        XCTAssertTrue(held.contains("summary?.envelopeDigest"), "and from the body on screen: \(held)")
        let review = try Self.declaration("private var nativeReview: some View {", in: sheet)
        XCTAssertFalse(review.contains("entry.holdsCertificate"), "the opening copy is stale after a review")
    }

    // MARK: - Nothing healthy-looking on a failure

    /// When the core's Look-inside table does not decode, or a summary
    /// arrives with no body, the content area says the one fixed sentence
    /// rather than drawing unnamed tabs or a blank pane.
    func test_aMissingTableOrBodyIsSaid() throws {
        let sheet = Self.code(try Self.sheet())
        let content = try Self.declaration("private var content: some View {", in: sheet)
        XCTAssertTrue(content.contains("} else if summary != nil, let words, let document {"),
                      "the tabs are drawn only with the core's words and a body: \(content)")
        XCTAssertTrue(content.contains("} else if summary != nil {"), "a summary with no body is said: \(content)")
        XCTAssertTrue(content.contains("SheetNotice(title: Self.cannotShow,"))
        XCTAssertFalse(content.contains("words?.title ?? tab.title(words)"))
    }

    /// The cannot-show sentence is the core's (`session_review`'s
    /// `cannot_show_title`). The literal is only the fallback for a table
    /// that did not decode, verbatim the core's, so the notice is never
    /// drawn without a title (the `HealthCopy.onHoldFallback` precedent).
    func test_theCannotShowSentenceIsTheCores() throws {
        let sheet = Self.code(try Self.sheet())
        let cannotShow = try Self.declaration("private static var cannotShow: String {", in: sheet)
        XCTAssertTrue(cannotShow.contains("Self.traces?.sessionReview.cannotShowTitle ?? Self.cannotShowFallback"),
                      "the sentence is not the core's: \(cannotShow)")
        let core = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        XCTAssertEqual(PreviewSheet.cannotShowFallback, core.sessionReview.cannotShowTitle)
    }

    /// Loading with no Look-inside table has no loading line to say: it
    /// says it cannot show the session, never an empty notice.
    func test_aMissingTableIsNeverAnEmptyNotice() throws {
        let sheet = Self.code(try Self.sheet())
        let content = try Self.declaration("private var content: some View {", in: sheet)
        XCTAssertFalse(content.contains("detail: words?.loadingTranscript ?? \"\""),
                       "a missing table draws an empty notice: \(content)")
        XCTAssertFalse(content.contains("detail: \"\""), "an empty notice: \(content)")
        let loading = try Self.declaration("} else if loading {", in: content)
        XCTAssertTrue(loading.contains("if let words {"), "the loading line needs the table: \(loading)")
        XCTAssertTrue(loading.contains("SheetNotice(title: Self.cannotShow, detail: cannotShowDetail)"),
                      "and without it the cannot-show notice: \(loading)")
    }

    /// `failure` can hold raw local error text (the preview build's own
    /// message); it never reaches a screen. A failure that is not a
    /// witness refusal reads the core's cannot-show detail.
    func test_aFailureNeverShowsLocalErrorText() throws {
        let sheet = Self.code(try Self.sheet())
        let content = try Self.declaration("private var content: some View {", in: sheet)
        for leak in [": failure", "?? failure", "detail: failure", "failure,"] {
            XCTAssertFalse(content.contains(leak), "`\(leak)` puts local error text on screen: \(content)")
        }
        XCTAssertTrue(content.contains(": cannotShowDetail"), "a non-witness failure reads the core's detail: \(content)")
        let detail = try Self.declaration("private var cannotShowDetail: String {", in: sheet)
        XCTAssertTrue(detail.contains("sessionReview.cannotShowBody"), "the detail is the core's: \(detail)")
        let core = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        XCTAssertFalse(core.sessionReview.cannotShowBody.isEmpty)
    }

    // MARK: - Turn index

    /// The turn index is the core's, anchored to the digest of the very
    /// body on screen, and every word in a row is the core's.
    func test_turnIndexReadsTheCore() throws {
        // The anchor: sha256 over the body's UTF-8 bytes, lowercase hex.
        XCTAssertEqual(LookInside.bodyDigest("abc"),
                       "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")

        let words = try XCTUnwrap(Self.words, "the core's Look-inside table did not decode")
        let json = """
            {"entry_id":"e1","body_digest":"sha256:00","envelope_digest":"sha256:11","turn_count":2,
             "turns":[{"index":0,"role":"user_message","byte_offset":0,"byte_len":10},
                      {"index":1,"role":"tool_call","tool_name":"Bash","byte_offset":10,"byte_len":6}]}
            """
        let turns = try XCTUnwrap(PreviewTurns.decode(fromJSON: json))
        XCTAssertEqual(LookInside.turnTitle(turns.turns[0]), "1. user message")
        XCTAssertEqual(LookInside.turnDetail(turns.turns[0], words: words),
                       words.turnEvent + " · " + FirstRunCopy.fill(words.turnBytes, ["start": "0", "end": "10"]))
        XCTAssertEqual(LookInside.turnDetail(turns.turns[1], words: words),
                       "Bash · " + FirstRunCopy.fill(words.turnBytes, ["start": "10", "end": "16"]))
        XCTAssertFalse(LookInside.turnDetail(turns.turns[1], words: words).contains("{"), "every hole is filled")

        // Separators split a chunk exactly at each turn that opens in it.
        let chunk = TranscriptDocument.Chunk(byteOffset: 4, byteCount: 12, lineCount: 1)
        let segments = LookInside.segments(of: chunk, turns: turns.turns)
        XCTAssertEqual(segments.map(\.byteRange), [4..<10, 10..<16])
        XCTAssertEqual(segments.map { $0.turn?.index }, [nil, 1])
        XCTAssertEqual(LookInside.segments(of: chunk, turns: []).map(\.byteRange), [4..<16])

        let sheet = Self.code(try Self.sheet())
        XCTAssertTrue(sheet.contains("model.previewTurns(entryID: entry.entryID, bodyDigest: digest)"))
        XCTAssertTrue(sheet.contains(".indexes(bodyDigest: digest)"), "an index for another body is never drawn")
        XCTAssertTrue(sheet.contains("LookInside.bodyDigest("))
        for needle in ["words.turnIndexEyebrow", "words.loadTurnIndex", "words.addTurnSeparators",
                       "words.turnsNeedFullRead", "LookInside.turnTitle(", "LookInside.turnDetail("] {
            XCTAssertTrue(sheet.contains(needle), "the turn index lacks \(needle)")
        }
        let client = try Self.text("DaemonClient.swift")
        XCTAssertTrue(client.contains("TCPreviewTurns.turnsJSON("), "the index is read through the core's bridge")
    }

    /// The transcript is shown a page at a time, Ron's Load more naming
    /// what is left, and the turn index waits for the whole of it.
    func test_loadMoreRevealsTheBodyByPage() {
        let document = TranscriptDocument(String(repeating: "x\n", count: 40_000))
        let page = 16 * 1024
        let first = LookInside.shownChunks(document, pages: 1, pageBytes: page)
        XCTAssertGreaterThan(first, 0)
        XCTAssertLessThan(first, document.chunkCount)
        let shownBytes = document.chunks[0..<first].reduce(0) { $0 + $1.byteCount }
        XCTAssertGreaterThanOrEqual(shownBytes, page)
        XCTAssertEqual(LookInside.remainingBytes(document, shownChunks: first), document.totalBytes - shownBytes)
        XCTAssertEqual(LookInside.shownChunks(document, pages: 1_000, pageBytes: page), document.chunkCount)
        XCTAssertEqual(LookInside.remainingBytes(document, shownChunks: document.chunkCount), 0)
        XCTAssertGreaterThan(LookInside.shownChunks(document, pages: 2, pageBytes: page), first)
    }

    // MARK: - Native extras kept

    /// Native's search of the redacted text and Copy everything stay,
    /// beside Ron's tabs; Command-F still goes to that search.
    func test_searchRedactedAndCopyEverythingRemain() throws {
        let sheet = Self.code(try Self.sheet())
        for needle in [
            "preview.search(needle)", "RecentSearches.remember(", "document.snippet(around:",
            "\"transcript-copy-all\"", "document.wholeText()", ".keyboardShortcut(\"f\", modifiers: .command)",
            "case transcript, searchOriginal, turnIndex, search",
        ] {
            XCTAssertTrue(sheet.contains(needle), "PreviewSheet.swift lacks \(needle)")
        }
        // Ron's original-session search: a count, never text.
        for needle in ["model.searchOriginal(entryID: entry.entryID, needle:", "words.checkCount",
                       "words.searchCaption", "LookInside.originalMatches(matches, words: words)"] {
            XCTAssertTrue(sheet.contains(needle), "the original search lacks \(needle)")
        }
        let words = try XCTUnwrap(Self.words)
        XCTAssertEqual(LookInside.originalMatches(1, words: words), words.originalMatchOne)
        XCTAssertEqual(LookInside.originalMatches(3, words: words), FirstRunCopy.fill(words.originalMatches, ["count": "3"]))
        XCTAssertEqual(LookInside.originalMatches(0, words: words), FirstRunCopy.fill(words.originalMatches, ["count": "0"]))
    }
}
