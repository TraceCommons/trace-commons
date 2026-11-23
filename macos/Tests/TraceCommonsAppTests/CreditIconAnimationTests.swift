import AppKit
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

@MainActor
final class CreditIconAnimationTests: XCTestCase {
    private final class MockDockTilePresenter: DockTilePresenter {
        var tileSize: CGSize = GlassDockCoinBurst.defaultSize
        var backingScale: CGFloat = 1
        var icon: NSImage? = NSImage(size: GlassDockCoinBurst.defaultSize)
        var attentionCount = 0
        var shownFrames = 0
        var restoreCount = 0

        func baseIcon() -> NSImage? { icon }
        func requestAttention() { attentionCount += 1 }
        func show(_ image: NSImage) { shownFrames += 1 }
        func restore() { restoreCount += 1 }
    }

    func testAppModelIncrementsCreditArrivalCountOnlyOnLiveIncrease() {
        let model = AppModel()
        XCTAssertEqual(model.creditArrivalCount, 0)

        // First reading sets the baseline.
        model.recordBalanceStatus(BalanceStatus.parse(fromJSON: """
            {"state":"known","scale":9,"received_total_nanos":4000000000,"remaining_nanos":4000000000}
            """))
        XCTAssertEqual(model.creditArrivalCount, 0)

        // Same reading does not fire.
        model.recordBalanceStatus(BalanceStatus.parse(fromJSON: """
            {"state":"known","scale":9,"received_total_nanos":4000000000,"remaining_nanos":3500000000}
            """))
        XCTAssertEqual(model.creditArrivalCount, 0)

        // Live increase fires.
        model.recordBalanceStatus(BalanceStatus.parse(fromJSON: """
            {"state":"known","scale":9,"received_total_nanos":6000000000,"remaining_nanos":5500000000}
            """))
        XCTAssertEqual(model.creditArrivalCount, 1)

        // Second live increase fires again.
        model.recordBalanceStatus(BalanceStatus.parse(fromJSON: """
            {"state":"known","scale":9,"received_total_nanos":9000000000,"remaining_nanos":8500000000}
            """))
        XCTAssertEqual(model.creditArrivalCount, 2)
    }

    func testAppModelIncrementsCreditArrivalCountOnTraceCommonsCreditsWithoutNearAiSignIn() {
        let model = AppModel()
        let zeroCounts = HistoryCounts(submitted: 0, accepted: 0, quarantined: 0, other: 0)
        func rollup(_ creditFinal: Double) -> HistoryRollup {
            HistoryRollup(
                week: zeroCounts, month: zeroCounts, allTime: zeroCounts,
                creditPending: 0, creditFinal: creditFinal, quarantined: 0, lastRefreshedAt: nil
            )
        }

        // First rollup establishes baseline without firing.
        model.recordHistoryRollup(rollup(1.0))
        XCTAssertEqual(model.creditArrivalCount, 0)

        // Unchanged credit does not fire.
        model.recordHistoryRollup(rollup(1.0))
        XCTAssertEqual(model.creditArrivalCount, 0)

        // Any settled credit fires, even with no NEAR AI balance.
        model.recordHistoryRollup(rollup(1.1))
        XCTAssertEqual(model.creditArrivalCount, 1)

        model.recordHistoryRollup(rollup(1.2))
        XCTAssertEqual(model.creditArrivalCount, 2)
    }

    func testDockAnimatorRunsFramesBouncesAndRestores() async {
        let presenter = MockDockTilePresenter()
        var sleptDurations: [Duration] = []
        let animator = DockCreditAnimator(
            presenter: presenter,
            reduceMotion: { false },
            sleep: { duration in
                sleptDurations.append(duration)
                await Task.yield()
            }
        )

        animator.celebrate()
        // Wait for the Task to finish all frames and call restore().
        for _ in 0..<200 where presenter.restoreCount == 0 {
            await Task.yield()
        }

        let expectedFrames = BalanceArrival.IconAnimation.frameTimes(reduceMotion: false).count
        XCTAssertEqual(presenter.attentionCount, 1)
        XCTAssertEqual(presenter.shownFrames, expectedFrames)
        XCTAssertEqual(sleptDurations.count, expectedFrames)
        XCTAssertEqual(presenter.restoreCount, 1)
    }

    func testDockAnimatorRespectsReduceMotion() async {
        let presenter = MockDockTilePresenter()
        var sleptDurations: [Duration] = []
        let animator = DockCreditAnimator(
            presenter: presenter,
            reduceMotion: { true },
            sleep: { duration in
                sleptDurations.append(duration)
                await Task.yield()
            }
        )

        animator.celebrate()
        for _ in 0..<50 where presenter.restoreCount == 0 {
            await Task.yield()
        }

        XCTAssertEqual(presenter.attentionCount, 0, "Reduce Motion must not bounce the Dock icon")
        XCTAssertEqual(presenter.shownFrames, 1, "Reduce Motion renders one still frame")
        XCTAssertEqual(sleptDurations, [BalanceArrival.IconAnimation.reduceMotionHold])
        XCTAssertEqual(presenter.restoreCount, 1)
    }

    func testDockAnimatorStopCancelsAndRestoresImmediately() async {
        let presenter = MockDockTilePresenter()
        let animator = DockCreditAnimator(
            presenter: presenter,
            reduceMotion: { false },
            sleep: { _ in try? await Task.sleep(for: .seconds(10)) }
        )

        animator.celebrate()
        await Task.yield()
        animator.stop()

        XCTAssertEqual(presenter.restoreCount, 1)
    }

    func testDockAnimatorSkipsAnimationWhenDisabledInSettings() async {
        let presenter = MockDockTilePresenter()
        let animator = DockCreditAnimator(
            presenter: presenter,
            isEnabled: { false },
            reduceMotion: { false },
            sleep: { _ in await Task.yield() }
        )

        animator.celebrate()
        await Task.yield()

        XCTAssertEqual(presenter.attentionCount, 0)
        XCTAssertEqual(presenter.shownFrames, 0)
        XCTAssertEqual(presenter.restoreCount, 1)
    }

    func testCreditIconAnimationCopyIsPresentInCoreTable() throws {
        let words = try XCTUnwrap(MonitorWords.table?.homeHistory)
        XCTAssertFalse(words.creditIconAnimationTitle.isEmpty)
        XCTAssertFalse(words.creditIconAnimationToggle.isEmpty)
        XCTAssertFalse(words.creditIconAnimationCaption.isEmpty)
    }
}
