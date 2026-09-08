import TCBridge
import XCTest

/// The settings screen's session-source rows, checked against the real dylib.
///
/// This cannot live in `TCShellCoreTests`: that target deliberately does not
/// link the FFI, and a fixture there would only assert that this file agrees
/// with itself. What has to be true is that the Rust picks a different
/// sentence for each of the three modes.
final class SourceCheckExportTests: XCTestCase {
    private func line(
        _ tool: String, _ mode: String, file: StaticString = #filePath, line: UInt = #line
    ) -> String? {
        guard let text = TCSourceChecks.checkLine(tool: tool, sourceMode: mode) else {
            XCTFail("the export refused \(tool)/\(mode)", file: file, line: line)
            return nil
        }
        return text
    }

    /// The sentences this shell will print, pinned as literals.
    ///
    /// The defect they pin: `off` and `unset` shared one sentence, because
    /// the row branched on `*_root_configured`, which is `mode == "watch"`
    /// and so false for both. A contributor who declared Claude Code off was
    /// told its sessions were being read from the usual place.
    ///
    /// `unset` keeps saying sessions are read, because they are -- an
    /// undeclared claude or codex source is scanned at its conventional
    /// location. Saying otherwise would be the same bug pointing the other
    /// way, and worse.
    func testEachSourceModeGetsItsOwnSentence() {
        XCTAssertEqual(
            line(TCSourceChecks.claude, "watch"), "Claude Code sessions folder set")
        XCTAssertEqual(
            line(TCSourceChecks.claude, "unset"),
            "Claude Code sessions read from the usual place")
        XCTAssertEqual(
            line(TCSourceChecks.claude, "off"),
            "Claude Code marked not used, so nothing is opened for it. Previously queued sessions are not removed")
        XCTAssertEqual(
            line(TCSourceChecks.codex, "off"),
            "Codex marked not used, so nothing is opened for it. Previously queued sessions are not removed")
    }

    /// No mode's sentence contains another's. "Private" is a substring of
    /// "Not private", and a `contains` on this surface has matched the wrong
    /// branch that way before; the `off` line is not the `unset` line with a
    /// negation bolted on.
    func testNoModesSentenceContainsAnothers() {
        for tool in [TCSourceChecks.claude, TCSourceChecks.codex] {
            let lines = ["watch", "unset", "off"].compactMap { line(tool, $0) }
            XCTAssertEqual(lines.count, 3)
            for a in lines {
                for b in lines where a != b {
                    XCTAssertFalse(
                        b.contains(a), "one mode's sentence contains another's: \(b) / \(a)")
                }
            }
            XCTAssertEqual(Set(lines).count, 3, "two modes render the same sentence")
        }
    }

    /// A mode this build does not know reads as `unset`, never as `off`. An
    /// older daemon sends no `*_source_mode` at all and this shell defaults
    /// it to "unset"; claiming nothing is read from a folder that is being
    /// scanned is the worse of the two errors.
    func testAnUnknownModeNeverClaimsNothingIsRead() {
        let unset = line(TCSourceChecks.claude, "unset")
        for mode in ["", "OFF", "disabled", "watching"] {
            XCTAssertEqual(line(TCSourceChecks.claude, mode), unset)
        }
    }

    /// A tool key this build does not have is refused rather than answered
    /// with some other tool's sentence under this tool's heading.
    func testAnUnknownToolIsRefused() {
        XCTAssertNil(TCSourceChecks.checkLine(tool: "claude-code", sourceMode: "watch"))
        XCTAssertNil(TCSourceChecks.checkLine(tool: "", sourceMode: "off"))
    }
}

extension SourceCheckExportTests {
    func testSettingsMetadataFollowsActualUndeclaredSourcePolicy() throws {
        let copy = try XCTUnwrap(TCSourceChecks.settingsCopy())
        // Derived from the payload, not hand-listed. A hand-written set is
        // how this broke: `opencode` became the fifth source and the four
        // named here silently became wrong. The property worth pinning is
        // that the Swift side sees exactly what Rust registered -- the count
        // guards against a source vanishing, and the membership checks below
        // are keyed off the payload rather than a literal.
        XCTAssertFalse(copy.tools.isEmpty, "the settings copy must carry the registered sources")
        XCTAssertTrue(
            copy.tools.keys.contains("claude-code") && copy.tools.keys.contains("codex"),
            "the two conventional-scan sources must always be present, got \(Set(copy.tools.keys))"
        )
        for (adapter, tool) in copy.tools {
            XCTAssertEqual(tool.unsetScansConventional, adapter == "claude-code" || adapter == "codex")
            let unset = try XCTUnwrap(TCSourceChecks.checkLine(tool: tool.key, sourceMode: "unset"))
            XCTAssertEqual(unset.contains("read from the usual place"), tool.unsetScansConventional)
            let off = try XCTUnwrap(TCSourceChecks.checkLine(tool: tool.key, sourceMode: "off"))
            XCTAssertTrue(off.contains("Previously queued sessions are not removed"))
        }
        XCTAssertTrue(copy.explanation.contains("does not remove sessions already queued"))
        XCTAssertTrue(copy.consentSaveFailed.contains("Couldn't confirm"))
    }
}
