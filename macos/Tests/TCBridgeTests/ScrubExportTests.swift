import TCBridge
import TCShellCore
import XCTest

/// The two exports onboarding depends on, checked against the real dylib.
///
/// These cannot live in `TCShellCoreTests`: that target deliberately does not
/// link the FFI, and the properties here are only meaningful against the
/// actual detector table. A fixture would assert that this file agrees with
/// itself.
final class ScrubExportTests: XCTestCase {
    func testEveryDetectorHasAHumanLabel() {
        guard let json = TCScrubInfo.detectorNamesJSON() else {
            return XCTFail("the detector export returned nil")
        }
        let slugs = ScrubDetectors.slugs(fromJSON: json)
        XCTAssertFalse(
            slugs.isEmpty,
            "an empty list would tell a contributor nothing is scrubbed"
        )

        for slug in slugs {
            let label = ScrubDetectors.label(for: slug)
            // The de-slugged fallback is what an unlabelled detector renders
            // as. It is a safety net so a new detector cannot VANISH from the
            // screen -- but it is not the plan, and a detector reaching a
            // contributor as "npm token" rather than "npm tokens" means
            // someone added one upstream and this table was not updated.
            XCTAssertNotEqual(
                label,
                slug.replacingOccurrences(of: "_", with: " "),
                "detector \(slug) has no human label in ScrubDetectors.label"
            )
        }
    }

    func testTheDetectorListIsNotEmptyAndCarriesNoPatterns() {
        guard let json = TCScrubInfo.detectorNamesJSON() else {
            return XCTFail("the detector export returned nil")
        }
        for slug in ScrubDetectors.slugs(fromJSON: json) {
            for meta in ["\\", "^", "$", "(", "[", "{", "+", "?", "|", "*"] {
                XCTAssertFalse(
                    slug.contains(meta),
                    "a regex metacharacter \(meta) reached the shell in \(slug)"
                )
            }
        }
    }

}
