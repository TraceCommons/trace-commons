import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The roots screen must offer a row for every source the contributor
/// library knows how to read.
///
/// This exists because it did not. The screen hardcoded Claude Code and
/// Codex, so when `gemini-cli` was added to the Rust side's source registry
/// the macOS app kept asking about two of the three. Nothing failed: the
/// Gemini candidate was fetched by `discover()` along with the other two and
/// then silently dropped for want of a row to render it in.
///
/// The silence is the point. `gemini-cli` is `Undeclared::Nothing` on the
/// Rust side, so an absent declaration constructs no adapter and reads
/// nothing -- deliberately, so that shells which shipped before the source
/// existed do not suddenly start scanning a contributor's real `~/.gemini`.
/// And `SessionRoots.isComplete` is two-conjunct by design, so an
/// unanswered Gemini row cannot block Continue either. Every mechanism that
/// would otherwise have surfaced the omission is one that was correctly
/// designed to stay quiet, which left the screen simply never asking.
final class OnboardingRootsRowsTests: XCTestCase {
    /// Pinned against `allCases` rather than against a written-out list, so
    /// that a fourth adapter cannot repeat this by being added to the enum
    /// and forgotten here.
    func testOffersARowForEverySourceKind() {
        XCTAssertEqual(
            OnboardingRootsView.offeredKinds,
            SourceKind.allCases,
            "the roots screen must offer every source the library can read -- "
                + "a kind missing here is never asked about and never watched"
        )
    }

    func testOffersGeminiSpecifically() {
        XCTAssertTrue(
            OnboardingRootsView.offeredKinds.contains(.geminiCli),
            "Gemini CLI has a row in the source registry, a settings key "
                + "(gemini_source) and a field on SessionRoots, but was never "
                + "offered on the one screen that can answer for it"
        )
    }

    /// Order is part of the contract: discovery appends Gemini last
    /// deliberately, because a shell written before the source existed
    /// indexes the first two candidates by position.
    func testClaudeAndCodexComeFirst() {
        XCTAssertEqual(OnboardingRootsView.offeredKinds.first, .claudeCode)
        XCTAssertEqual(OnboardingRootsView.offeredKinds.dropFirst().first, .codex)
    }
}
