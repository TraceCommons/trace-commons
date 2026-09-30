import CTraceCommons
import XCTest

@testable import TCBridge

/// The login-enrolment sentences, taken from the live ABI rather than a
/// fixture, so they cannot go stale when the ABI moves.
final class TCNearAiEnrollTests: XCTestCase {
    private static let labels = [
        "near_ai_enroll_already_enrolled",
        "near_ai_enroll_no_session",
        "near_ai_enroll_endpoint_refused",
        "near_ai_enroll_token_unavailable",
        "near_ai_enroll_start_failed",
        "near_ai_enroll_commons_unreachable",
        "near_ai_enroll_commons_unsupported",
        "near_ai_enroll_invalid",
        "near_ai_enroll_verification_failed",
        "near_ai_enroll_unavailable",
    ]

    /// Ten refusals, ten sentences, none silent.
    func testEveryRefusalReachesItsOwnSentence() throws {
        var seen = Set<String>()
        for label in Self.labels {
            let line = try XCTUnwrap(TCNearAiEnroll.line(label: label))
            XCTAssertFalse(line.isEmpty, "\(label) reached no sentence")
            XCTAssertTrue(seen.insert(line).inserted, "\(label) shares a sentence with another")
        }
    }

    /// An unfamiliar label reaches the generic sentence, never nothing and
    /// never another refusal's words.
    func testAnUnknownLabelClaimsNothingSpecific() throws {
        let generic = try XCTUnwrap(TCNearAiEnroll.line(label: "near_ai_enroll_unavailable"))
        for unknown in ["", "near_ai_enroll_from_a_newer_daemon"] {
            XCTAssertEqual(TCNearAiEnroll.line(label: unknown), generic)
        }
    }

    /// The two outcomes a contributor would misread if the tone were wrong.
    ///
    /// Not being signed in yet is a step to take, not a wall; being already
    /// joined is the outcome they wanted, not a failure.
    func testTheTonesPointAtTheStepRatherThanTheWall() {
        XCTAssertEqual(
            TCNearAiEnroll.tone(label: "near_ai_enroll_no_session"),
            TC_PRIVATE_INFERENCE_TONE_ATTENTION,
            "a contributor who has simply not signed in was shown a refusal")
        XCTAssertEqual(
            TCNearAiEnroll.tone(label: "near_ai_enroll_already_enrolled"),
            TC_PRIVATE_INFERENCE_TONE_CLEAR,
            "being already joined was painted as a failure")
        XCTAssertEqual(
            TCNearAiEnroll.tone(label: "near_ai_enroll_commons_unreachable"),
            TC_PRIVATE_INFERENCE_TONE_REFUSED)
    }
}
