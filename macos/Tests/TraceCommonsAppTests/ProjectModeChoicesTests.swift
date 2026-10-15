import TCBridge
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

final class ProjectModeChoicesTests: XCTestCase {
    /// The core's contribution-mode table names every ProjectMode the daemon
    /// accepts, by the daemon's own mode string.
    func test_theCoreNamesEveryProjectMode() throws {
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        for mode in [ProjectMode.ask, .autoUpload, .ignore] {
            XCTAssertNotNil(copy.choice(for: mode.rawValue), "no core label for \(mode.rawValue)")
        }
    }

    /// The picker offers only the modes the row can take, in the row's
    /// order, each labelled by the core.
    func test_optionsFollowOfferableModes() throws {
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        let options = ProjectModeChoices.options(for: [.ask, .ignore], copy: copy)
        XCTAssertEqual(options.map(\.value), [.ask, .ignore])
        XCTAssertEqual(options.map(\.title), [copy.choice(for: "notify_only")?.label, copy.choice(for: "ignore")?.label])
    }

    /// A mode the core's table does not name is not offered, rather than
    /// offered under a word this shell made up.
    func test_aModeTheCoreDoesNotNameIsNotOffered() throws {
        let json = """
            {"title":"t","mixed":"m","choices":[{"mode":"ignore","label":"Never","line":"l"}],
             "override_active":"o","clear":"c","auto_partial":"a"}
            """
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: json))
        let options = ProjectModeChoices.options(for: [.ask, .autoUpload, .ignore], copy: copy)
        XCTAssertEqual(options.map(\.value), [.ignore])
    }
}
