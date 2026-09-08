import Foundation
import XCTest
@testable import TraceCommonsApp

/// Both of the model's one-line action messages must be dismissible.
///
/// `lastActionError` got its dismiss control in PR #639 and `lastActionNotice`
/// did not, so a notice -- the reconciliation sentence after ignoring a
/// project, or the withheld-count sentence after an approval -- stayed on
/// screen for the rest of the session with no way out. Nothing in the model
/// ever assigns `nil` to it either, so there is no other path that clears it.
///
/// Read as a source scan rather than by pressing the button, and deliberately.
/// SwiftUI builds its accessibility tree only when the system's accessibility
/// server asks for it, from another process; under `swift test` an
/// `NSHostingView` reports zero accessibility children, so a hosted "press the
/// x" test would pass and fail for reasons that have nothing to do with this
/// defect. Reading the render sites is the same guard `ShellWordingTests`
/// applies to this shell's wording, for the same reason.
///
/// ## The sweep, so it is not run again from scratch
///
/// Every other piece of state this shell shows a contributor and holds in a
/// model was checked for the same shape -- assigned, shown, never cleared --
/// and `lastActionNotice` was the only one. The near misses, and why each is
/// a different thing:
///
/// - `AppModel.profileOutcome`, `AppModel.routingProbeLine`,
///   `AppModel.withdrawals[id]`, `AppModel.inferenceEvidenceSaveFailed` and
///   `ComputeModel.failureLabel` are all set to `nil`/`false` at the START of
///   the action that produces them. They are the standing result of a check a
///   person just asked for -- a readout, not a one-shot announcement -- and
///   the next attempt replaces them. Dismissing a readout would leave the
///   surface saying nothing about a state that still holds.
/// - `AppModel.summaryErrors[id]`, `credentialAttempt` and
///   `harnessExposureRequest` are each cleared on their own completion path.
/// - `SettingsView.loginItemActionError`, `SettingsView.consentSaveError`,
///   `OnboardingRootsView.failure` and `PreviewSheet.failure` are view-local
///   `@State`, cleared at the top of each attempt and gone with the view.
///
/// What made the notice different is that only two actions ever assign it and
/// nothing anywhere assigns `nil`, so nothing in the app's own operation could
/// ever take it off the screen.
final class ActionNoticeDismissTests: XCTestCase {
    /// The published properties that reach a contributor as a banner.
    private static let messageProperties = ["lastActionError", "lastActionNotice"]

    /// Every render of an action message goes through the dismissible banner,
    /// and each banner clears the very property whose presence drew it.
    ///
    /// The second half is what makes this more than a spelling check: a banner
    /// wired to clear the other message would still draw an x, would still
    /// look right in a screenshot, and would leave the sentence on screen.
    func testEveryActionMessageRendersThroughABannerThatClearsThatSameProperty() throws {
        let sources = try Self.appSources()

        // A scan that found nothing would turn this test into a pass over
        // nothing. There are 40+ Swift sources under the app target today.
        XCTAssertGreaterThanOrEqual(
            sources.count, 30,
            "only \(sources.count) sources were scanned; the whole app target is expected")

        var sites: [String: Int] = [:]
        var failures: [String] = []
        for (path, text) in sources.sorted(by: { $0.key < $1.key }) {
            let lines = text.components(separatedBy: "\n")
            for (index, line) in lines.enumerated() {
                guard let property = Self.messageProperties.first(where: {
                    line.contains("if let") && line.contains("model.\($0)")
                }) else { continue }
                sites[property, default: 0] += 1
                let rendered = lines[(index + 1)...].prefix(3).joined(separator: " ")
                let location = "\(path):\(index + 1) (\(property))"
                if !rendered.contains("ActionMessageBanner(") {
                    failures.append(
                        "\(location) renders without a dismiss control: \(rendered.trimmed)")
                } else if !rendered.contains("model.\(property) = nil") {
                    failures.append(
                        "\(location) has a banner whose dismiss does not clear \(property): "
                            + rendered.trimmed)
                }
            }
        }

        // Preconditions on the scan itself. Both properties are rendered
        // somewhere today; if a rename hid one from this scan, the loop above
        // would have had nothing to check and would have said nothing.
        for property in Self.messageProperties {
            XCTAssertGreaterThan(
                sites[property] ?? 0, 0,
                "no render site was found for \(property) -- this scan proved nothing about it")
        }
        XCTAssertTrue(failures.isEmpty, failures.joined(separator: "\n"))
    }

    /// The dismiss closure's own precondition: the notice is externally
    /// settable to `nil`. Most of this model's state is `private(set)`, and a
    /// notice that became so would take the dismiss control with it.
    @MainActor
    func testTheNoticeCanBeClearedFromOutsideTheModel() {
        let model = AppModel() // No daemon or enrollment starts here.
        XCTAssertNil(model.lastActionNotice, "a fresh model carries no notice")
        model.lastActionNotice = "Synthetic project is ignored; 3 of the 4 it promised are gone."
        XCTAssertNotNil(model.lastActionNotice)
        model.lastActionNotice = nil
        XCTAssertNil(model.lastActionNotice)
    }

    /// `.../macos/Tests/TraceCommonsAppTests/<this file>` ->
    /// `.../macos/Sources/TraceCommonsApp`, located from this file's own path
    /// the way `ShellWordingTests` does: `swift test`'s working directory is
    /// not something to depend on.
    private static func appSources() throws -> [String: String] {
        let base = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // TraceCommonsAppTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .appendingPathComponent("Sources")
            .appendingPathComponent("TraceCommonsApp")
        let walker = try XCTUnwrap(
            FileManager.default.enumerator(at: base, includingPropertiesForKeys: nil))
        var scanned: [String: String] = [:]
        for case let url as URL in walker where url.pathExtension == "swift" {
            let relative = url.path.replacingOccurrences(of: base.path + "/", with: "")
            scanned[relative] = try String(contentsOf: url, encoding: .utf8)
        }
        return scanned
    }
}

extension String {
    fileprivate var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}
