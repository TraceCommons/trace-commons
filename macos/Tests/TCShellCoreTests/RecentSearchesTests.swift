import XCTest

@testable import TCShellCore

/// A recent-search list is the contributor's list of the things they were
/// afraid of leaking. It stays in memory for that reason, and it must hold
/// what they actually asked -- not every prefix they typed on the way there.
@MainActor
final class RecentSearchesTests: XCTestCase {
    override func setUp() {
        super.setUp()
        RecentSearches.reset()
    }

    func testAnEmptyListStartsEmpty() {
        XCTAssertTrue(RecentSearches.load().isEmpty)
    }

    func testACommittedTermIsRemembered() {
        XCTAssertEqual(RecentSearches.remember("acme-corp"), ["acme-corp"])
    }

    func testTheMostRecentTermLeads() {
        _ = RecentSearches.remember("first")
        XCTAssertEqual(RecentSearches.remember("second"), ["second", "first"])
    }

    func testRepeatingATermMovesItToTheFrontWithoutDuplicating() {
        _ = RecentSearches.remember("a")
        _ = RecentSearches.remember("b")
        XCTAssertEqual(RecentSearches.remember("a"), ["a", "b"])
    }

    func testTheListIsCappedAtSix() {
        for term in ["1", "2", "3", "4", "5", "6", "7"] {
            _ = RecentSearches.remember(term)
        }
        XCTAssertEqual(RecentSearches.load().count, 6)
        XCTAssertEqual(RecentSearches.load().first, "7")
        XCTAssertFalse(RecentSearches.load().contains("1"))
    }

    func testAnEmptyOrBlankTermIsNotRemembered() {
        _ = RecentSearches.remember("")
        _ = RecentSearches.remember("   ")
        XCTAssertTrue(RecentSearches.load().isEmpty)
    }
}
