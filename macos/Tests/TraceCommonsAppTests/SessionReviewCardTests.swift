#if DEBUG
import Foundation
import XCTest
import TCBridge
import TCDesign
@testable import TCShellCore
@testable import TraceCommonsApp

/// The session review card (Task 6 of the #1241 port of Ron's #1146): the
/// inspector card is the review. It carries the verdict, the correction
/// under Partly or Failed, and Contribute, which is the one approve control
/// in the Monitor.
@MainActor
final class SessionReviewCardTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    static let card = "Views/Monitor/SessionReviewCard.swift"

    /// Records every call, answers `approve` with `approveReply`, and
    /// refuses everything else, so the reload after a contribution reads
    /// nothing and the test sees exactly what was sent.
    private final class RecordingTransport: DaemonTransport, @unchecked Sendable {
        private let lock = NSLock()
        private var calls: [(method: String, params: String)] = []
        let approveReply: String

        init(approveReply: String) {
            self.approveReply = approveReply
        }

        func call(_ method: String, params paramsJSON: String) -> String {
            lock.withLock { calls.append((method, paramsJSON)) }
            if method == "approve" { return #"{"result":"# + approveReply + "}" }
            return #"{"error":{"code":"bad_params","message":"not-in-this-test"}}"#
        }

        /// The params of every `approve` sent, in order.
        var approves: [[String: Any]] {
            lock.withLock {
                calls.filter { $0.method == "approve" }.compactMap {
                    try? JSONSerialization.jsonObject(with: Data($0.params.utf8)) as? [String: Any]
                }
            }
        }
    }

    /// Contribute sends the card's verdict and its correction, trimmed. Review
    /// Focus 1: Partly or Failed with an empty correction sends the verdict and
    /// no correction key at all, never an empty one.
    func test_contributeSendsTheVerdictAndCorrection() async throws {
        let transport = RecordingTransport(approveReply: SampleDaemonData.approved)
        let store = TracesStore(client: LiveDaemonClient(transport: transport))

        var draft = SessionReviewDraft(entryId: "e1")
        draft.choose(.partly)
        draft.write("  It skipped the migration.  ", limit: 2000)
        await store.perform(.contribute, on: "e1", verdict: draft.verdict, correction: draft.correctionToSend)
        let sent = try XCTUnwrap(transport.approves.last)
        XCTAssertEqual(sent["entry_id"] as? String, "e1")
        XCTAssertEqual(sent["outcome"] as? String, "partly")
        XCTAssertEqual(sent["correction"] as? String, "It skipped the migration.")
        XCTAssertEqual(store.lastContributed?.entryId, "e1")

        var empty = SessionReviewDraft(entryId: "e2")
        empty.choose(.failed)
        empty.write("  \n ", limit: 2000)
        await store.perform(.contribute, on: "e2", verdict: empty.verdict, correction: empty.correctionToSend)
        let bare = try XCTUnwrap(transport.approves.last)
        XCTAssertEqual(bare["entry_id"] as? String, "e2")
        XCTAssertEqual(bare["outcome"] as? String, "failed")
        XCTAssertNil(bare["correction"], "a blank correction is no key, never an empty one")
        XCTAssertEqual(transport.approves.count, 2)

        // The card's Contribute is what sends the draft, and only the draft
        // for the session it is pressed on.
        let card = try Self.text(Self.card)
        XCTAssertTrue(card.contains(
            "store.perform(.contribute, on: entry.entryId, verdict: draft.verdict, correction: draft.correctionToSend)"))
        XCTAssertTrue(card.contains("let draft = self.draft.current(for: entry.entryId)"))
    }

    /// The correction field is drawn only under Partly or Failed, and what
    /// was written under them is never sent once the verdict moves to Worked
    /// or to none. The text is kept, though: the arrow keys step through
    /// Worked on the way from Partly to Failed, and that step never wipes
    /// what was typed. A draft for one session is never another session's.
    func test_theCorrectionShowsOnlyUnderPartlyOrFailed() throws {
        var draft = SessionReviewDraft(entryId: "A")
        XCTAssertNil(draft.verdict)
        XCTAssertFalse(draft.correctionOffered)
        draft.choose(.worked)
        XCTAssertEqual(draft.verdict, .worked)
        XCTAssertFalse(draft.correctionOffered)
        draft.choose(.partly)
        XCTAssertTrue(draft.correctionOffered)
        draft.choose(.failed)
        XCTAssertTrue(draft.correctionOffered)
        draft.write("It never ran the tests.", limit: 2000)
        XCTAssertEqual(draft.correctionToSend, "It never ran the tests.")
        draft.choose(.worked)
        XCTAssertFalse(draft.correctionOffered)
        XCTAssertNil(draft.correctionToSend, "a correction is never sent under Worked")
        draft.choose(.failed)
        XCTAssertEqual(draft.correctionToSend, "It never ran the tests.",
                       "passing through Worked keeps what was typed")
        draft.choose(.worked)
        // Choosing the chosen answer again takes it back.
        draft.choose(.worked)
        XCTAssertNil(draft.verdict)
        XCTAssertNil(draft.correctionToSend, "a correction is never sent with no answer")
        draft.choose(.partly)
        XCTAssertEqual(draft.correctionToSend, "It never ran the tests.")

        // The core's limit is held at the keyboard.
        var long = SessionReviewDraft(entryId: "A")
        long.choose(.partly)
        long.write(String(repeating: "x", count: 30), limit: 12)
        XCTAssertEqual(long.correction.count, 12)

        // Another session's draft is never sent for this one.
        XCTAssertEqual(long.current(for: "A"), long)
        XCTAssertEqual(long.current(for: "B"), SessionReviewDraft(entryId: "B"))

        // The field and its words are the core's, inside the Partly/Failed
        // branch only.
        let card = try Self.text(Self.card)
        let branch = try XCTUnwrap(card.range(of: "if draft.correctionOffered {"))
        let field = String(card[branch.lowerBound...])
        for needle in ["outcome.correctionQuestion", "outcome.correctionPlaceholder", "outcome.correctionCaption",
                       "outcome.maxCorrectionChars"] {
            XCTAssertTrue(field.contains(needle), "the correction field lacks \(needle)")
            XCTAssertFalse(String(card[..<branch.lowerBound]).contains(needle),
                           "\(needle) is drawn outside Partly or Failed")
        }
        for needle in ["outcome.verdictQuestion", "outcome.worked", "outcome.partly", "outcome.failed",
                       "outcome.verdictCaption"] {
            XCTAssertTrue(card.contains(needle), "the verdict lacks \(needle)")
        }
        XCTAssertFalse(card.contains("CorrectionCopy.caption"), "the caption is the core's, not the shell's")
        XCTAssertFalse(card.contains("VerdictCopy.question"), "the question is the core's, not the shell's")
    }

    /// A correction the daemon reads as holding a credential is refused:
    /// nothing is contributed, the card says the core's headline and body
    /// in place of the generic refusal, and the next press clears it.
    func test_aCredentialCorrectionIsRefusedAndNothingSent() async throws {
        let transport = RecordingTransport(approveReply: SampleDaemonData.approveSkipped(
            entryId: "e1", reason: CorrectionCopy.credentialRefusalLabel))
        let store = TracesStore(client: LiveDaemonClient(transport: transport))
        await store.perform(.contribute, on: "e1", verdict: .failed, correction: "my token is sk-live-0000")
        XCTAssertEqual(transport.approves.count, 1)
        XCTAssertNil(store.lastContributed, "a refused correction is never drawn as a contribution")
        XCTAssertNil(store.actionError, "the generic refusal is not said over the credential's")
        XCTAssertEqual(store.correctionRefused, "e1")

        // The words are the core's outcome table.
        let disclosure = try XCTUnwrap(store.disclosure)
        XCTAssertEqual(disclosure, ContributorDisclosureCopy.decode(fromJSON: TCCoreCopy.contributorDisclosureCopyJSON()))
        XCTAssertFalse(disclosure.outcome.correctionCredentialHeadline.isEmpty)
        let card = try Self.text(Self.card)
        let refusal = try XCTUnwrap(card.range(of: "if store.correctionRefused == entry.entryId"))
        let after = String(card[refusal.lowerBound...])
        XCTAssertTrue(after.contains("outcome.correctionCredentialHeadline"))
        XCTAssertTrue(after.contains("outcome.correctionCredentialBody"))
        XCTAssertFalse(card.contains("CorrectionCopy.credentialHeadline"))

        // A later press on the same session starts clean.
        let ok = RecordingTransport(approveReply: SampleDaemonData.approved)
        store.attach(LiveDaemonClient(transport: ok))
        await store.perform(.contribute, on: "e1", verdict: .failed, correction: nil)
        XCTAssertNil(store.correctionRefused)
        XCTAssertEqual(store.lastContributed?.entryId, "e1")
    }

    /// A card left up over a core that stopped answering (its banner says
    /// so) is not armed: Contribute is drawn disabled and a press sends
    /// nothing, whichever tab the card is on.
    func test_aCoreThatStoppedAnsweringDisarmsContribute() throws {
        XCTAssertFalse(SessionReviewCard.coreAnswering(.failed(.unreachable)))
        XCTAssertFalse(SessionReviewCard.coreAnswering(.failed(.undecodable(method: "list_pending"))))
        XCTAssertTrue(SessionReviewCard.coreAnswering(.loaded))
        let card = try Self.text(Self.card)
        let armed = try XCTUnwrap(card.range(of: "private func armed(_ entry: DaemonData.QueueEntry) -> Bool {"))
        let end = try XCTUnwrap(card.range(of: "\n    }\n", range: armed.upperBound..<card.endIndex))
        XCTAssertTrue(card[armed.upperBound..<end.lowerBound].contains("Self.coreAnswering(store.phase)"),
                      "Contribute is armed over a core that is down")
    }

    /// zmanian's follow-up on #1273: with the core down, Contribute's
    /// tooltip gives the core-down reason (the banner's title), not the
    /// consent gate's not-ready one.
    func test_aCoreThatStoppedAnsweringExplainsContribute() throws {
        let down = try XCTUnwrap(TracesHealth.coreDownLine?.title)
        XCTAssertEqual(SessionReviewCard.coreDownHelp, down)
        XCTAssertNotEqual(SessionReviewCard.coreDownHelp, TCConsentCopy.gateHelp(pinned: false))
        let card = try Self.text(Self.card)
        XCTAssertTrue(card.contains(".help(Self.coreAnswering(store.phase)\n"), "the tooltip does not check the core first")
        XCTAssertTrue(card.contains(": Self.coreDownHelp)"))
    }

    /// Native's Keep stays on the card, and its Undo stays in the prompts.
    func test_keepAndUndoKeepRemain() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let id = try XCTUnwrap(store.tree.allSessions.first?.entryId)
        await store.perform(.keep, on: id)
        XCTAssertEqual(store.lastKept, id)
        await store.perform(.undoKeep, on: id)
        XCTAssertNil(store.lastKept)
        XCTAssertNil(store.actionError?.error)

        let card = try Self.text(Self.card)
        for needle in ["Button(words.keep) { act(.keep, entry) }", "Button(words.dismissAction) { act(.dismiss, entry) }",
                       "Button(review.lookInside) {"] {
            XCTAssertTrue(card.contains(needle), "the card lacks \(needle)")
        }
        let prompts = try Self.text("Views/Monitor/InspectorPrompts.swift")
        XCTAssertTrue(prompts.contains("store.perform(.undoKeep, on: kept)"))
    }

    /// V2 and P16 of the #1146 delta: Ron's `InspectorHeader` (the session
    /// tile, the folder, "Session · tool") heads the card, and Look inside,
    /// Dismiss and Contribute end the review card on one line, Keep a link
    /// under them. Look inside is always offered, on this session's entry.
    func test_theCardIsRonsShape() async throws {
        let store = TracesStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let words = try XCTUnwrap(store.words)
        let entry = try XCTUnwrap(store.tree.allSessions.first)
        let sub = SessionReviewCard.headerSub(entry, words: words)
        XCTAssertTrue(sub.hasPrefix(FirstRunCopy.fill(words.inspector.sessionOf, ["tool": ""])), sub)
        XCTAssertFalse(sub.contains("{"), sub)

        // Look inside with the legacy queue empty: the same session.
        let carried = QueueEntryBridge.previewEntry(entry, in: [])
        XCTAssertEqual(carried.entryID, entry.entryId)
        XCTAssertEqual(carried.projectID, entry.projectId)
        XCTAssertEqual(carried.projectLabel, entry.projectLabel)
        XCTAssertEqual(carried.source, entry.source)
        XCTAssertEqual(carried.sizeBytes, entry.sizeBytes)
        XCTAssertEqual(carried.discoveredAt, entry.discoveredAt ?? entry.startedAt)
        // The legacy queue's own entry wins when it holds the session.
        let held = QueueEntryBridge.previewEntry(entry, in: [carried])
        XCTAssertEqual(held, carried)
        // #1273 review: a size or a time the daemon did not report is never
        // fabricated as zero bytes or the epoch.
        let source = try Self.text(Self.card)
        XCTAssertFalse(source.contains("entry.sizeBytes ?? 0"))
        XCTAssertFalse(source.contains("Date(timeIntervalSince1970: 0)"))
        XCTAssertTrue(source.contains("sizeBytes: entry.sizeBytes,\n            discoveredAt: entry.discoveredAt ?? entry.startedAt,"))

        let card = try Self.text(Self.card)
        for needle in [
            "InspectorHeader(\n                            tile: .session, title: entry.projectLabel,",
            "previewing = QueueEntryBridge.previewEntry(entry, in: model.awaitingDecision)",
            ".buttonStyle(GlassButtonStyle(.primary, small: true))",
        ] {
            XCTAssertTrue(card.contains(needle), "the card lacks \(needle)")
        }
        // The buttons are inside the review card, after the preview.
        let preview = try XCTUnwrap(card.range(of: "preview(entry, review)\n                                    actions(entry)"))
        let kept = try XCTUnwrap(card.range(of: "keptLines(entry)\n"))
        XCTAssertLessThan(preview.lowerBound, kept.lowerBound)
        XCTAssertFalse(card.contains("Text(summary?.title ?? TracesTreeView.when(entry))"), "the old title heads the card")
        // One line, left-aligned: Contribute first, then Dismiss and Look
        // inside as links (Ron, 2026-10-09), none wrapping or shortened;
        // only when Contribute's label is too long for the line does Look
        // inside move down a line. Keep sits under.
        let actions = try XCTUnwrap(card.range(of: "private func actions("))
        let body = String(card[actions.lowerBound...])
        let flat = body.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        XCTAssertTrue(flat.contains(
            "ViewThatFits(in: .horizontal) { HStack(spacing: GlassTokens.Space.s4) {"
                + " contribute(entry, words) dismiss(entry, words) lookInside(entry, review) }"),
            "the one-line row comes first, Contribute leading")
        XCTAssertTrue(flat.contains(
            "HStack(spacing: GlassTokens.Space.s4) { contribute(entry, words) dismiss(entry, words) }"
                + " lookInside(entry, review) }"),
            "the two-line form keeps Contribute first and moves Look inside down")
        XCTAssertTrue(flat.contains(
            "Button(words.dismissAction) { act(.dismiss, entry) } .buttonStyle(GlassButtonStyle(.link))"),
            "Dismiss is a link after the action")
        XCTAssertTrue(flat.contains(".frame(maxWidth: .infinity, alignment: .leading) .disabled(busy)"),
                      "the buttons are left-aligned")
        let oneLine = try XCTUnwrap(body.range(of: "HStack(spacing: GlassTokens.Space.s4) {\n                        contribute"))
        let twoLines = try XCTUnwrap(body.range(of: "VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {\n                        HStack"))
        XCTAssertLessThan(oneLine.lowerBound, twoLines.lowerBound, "ViewThatFits tries the one line first")
        let fits = try XCTUnwrap(body.range(of: "ViewThatFits("))
        let keep = try XCTUnwrap(body.range(of: "Button(words.keep)"))
        XCTAssertLessThan(fits.lowerBound, keep.lowerBound, "Keep sits under the row")
        // A refusal is said under the buttons, never above them.
        let refusal = try XCTUnwrap(body.range(of: "TracesRefusal(store: store, entryId: entry.entryId)"))
        XCTAssertLessThan(keep.lowerBound, refusal.lowerBound, "the refusal sits above the buttons")
        for button in ["Button(review.lookInside)", "Button(words.dismissAction)", "Button(Self.contributeLabel("] {
            let start = try XCTUnwrap(body.range(of: button))
            let tail = body[start.upperBound...].prefix(600)
            XCTAssertTrue(tail.contains(".lineLimit(1)") && tail.contains(".fixedSize()"), "\(button) can wrap or clip")
        }
    }

    /// The token distribution line, which the deleted What's-in-it tab drew,
    /// is one of the card's kept native facts, and lives only there. It is
    /// the daemon's sentence, drawn only when the daemon sent one.
    func test_theTokenDistributionLineIsOnTheCard() throws {
        let card = try Self.text(Self.card)
        let kept = try XCTUnwrap(card.range(of: "private func keptLines"))
        let actions = try XCTUnwrap(card.range(of: "// MARK: Actions"))
        let body = String(card[kept.lowerBound..<actions.lowerBound])
        XCTAssertTrue(body.contains("if let line = summary?.tokenDistributionSummary, !line.isEmpty"),
                      "the card's kept facts lack the token distribution line")
        XCTAssertTrue(body.contains("caption(line)"), "the token distribution line is not drawn as a caption")
        let sheet = try Self.text("Views/PreviewSheet.swift")
        XCTAssertFalse(sheet.contains("tokenDistributionSummary"), "Look inside still draws the token distribution")
    }

    /// #1146's answers and chip: Worked / Partly / Failed are standard glass
    /// buttons and the chosen one is the selected (purple) glass button;
    /// the question is 12/600; Enrolled is the glass chip with no dot, and
    /// Not enrolled the muted one.
    func test_theVerdictAndChipAreRons() throws {
        let card = try Self.text(Self.card)
        // The answers are radios, one at most (owner, 2026-10-10).
        XCTAssertTrue(card.contains("GlassRadioRow(\n                outcome.verdictQuestion, selection: draft.verdict,"))
        XCTAssertFalse(card.contains(".buttonStyle(GlassButtonStyle(.glass, selected: selected))"), "the pressed-button answers are back")
        XCTAssertFalse(card.contains(".glassTier(selected ? .controlSelected : .control)"), "the custom verdict pill is back")
        XCTAssertFalse(card.contains("Image(systemName: \"checkmark\")"), "the chosen answer is a radio, not a mark")
        XCTAssertTrue(card.contains(".glassType(GlassTokens.TypeScale.label.weight(.semibold))"))
        XCTAssertTrue(card.contains("GlassChip(glass: summary.enrolled == true ? review.enrolled : review.notEnrolled,"))
        XCTAssertTrue(card.contains("muted: summary.enrolled != true)"), "unknown enrolment reads as not enrolled")
        XCTAssertFalse(card.contains("status: summary.enrolled == true ? .on : .ask"), "the green-dot status chip is back")
    }

    /// Contribute on the card is the one approve control in the Monitor:
    /// no other Monitor file contributes a single session, and the store's
    /// single `approve` is the only one that reaches the daemon.
    func test_contributeIsTheOnlyApproveControl() throws {
        let dir = GlassSurfaceRulesTests.root.appendingPathComponent("Views/Monitor")
        let files = try FileManager.default.contentsOfDirectory(atPath: dir.path).filter { $0.hasSuffix(".swift") }
        XCTAssertTrue(files.contains("SessionReviewCard.swift"))
        var contributes: [String: Int] = [:]
        var approves: [String: Int] = [:]
        for file in files {
            let source = try Self.text("Views/Monitor/\(file)")
            contributes[file] = source.components(separatedBy: ".perform(.contribute,").count - 1
            approves[file] = source.components(separatedBy: ".approve(entryId:").count - 1
            XCTAssertFalse(source.contains("model.approve("), "\(file) approves through the legacy model")
        }
        XCTAssertEqual(contributes.filter { $0.value > 0 }, ["SessionReviewCard.swift": 1],
                       "only the card's Contribute contributes a session")
        XCTAssertEqual(approves.filter { $0.value > 0 }, ["TracesStore.swift": 1],
                       "the store's Contribute is the one approve call")
        let host = try TracesInspectorHostTests.hostBody()
        XCTAssertTrue(host.contains("SessionReviewCard(store: traces, entry: entry)"))
        XCTAssertTrue(GlassSurfaceRulesTests.files.contains(Self.card), "the card is not under the glass rules")
        try LegacySymbols.assertClean(Self.card)
    }
}
#endif
