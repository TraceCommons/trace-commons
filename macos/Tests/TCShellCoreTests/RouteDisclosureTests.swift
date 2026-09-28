import TCShellCore
import XCTest

/// K11's decoder, against stand-in words: which blocks are present is what
/// matters here, not what they say.
final class RouteDisclosureTests: XCTestCase {
    private func payload(
        route: String = "witness",
        witness: Bool = true,
        classifier: Bool = true,
        localFilter: String? = nil,
        receipts: Bool = true,
        attestedBodies: Bool = false
    ) -> String {
        var facts: [String: Any] = [
            "route": route,
            "local_filter": route == "local" ? "near_ai" : NSNull(),
            "receipts": ["endpoint_configured": false, "check_attestation": false],
            "attested_bodies": attestedBodies,
        ]
        facts["witness"] = witness
            ? [
                "state": "pinned", "url": "https://witness.invalid", "signing_address": "0xab",
                "pinned_measurements": ["mrtd=aa"], "origin": "published_at_join",
            ] as [String: Any]
            : NSNull()
        let copy: [String: Any] = [
            "title": "TITLE",
            "route": "ROUTE",
            "witness": witness
                ? [
                    "heading": "W", "address_label": "A", "signing_label": "S",
                    "measurements_label": "M", "check": "CHECK",
                    "classifier": classifier ? "CLASSIFIER" : NSNull(), "origin": "ORIGIN",
                ] as [String: Any]
                : NSNull(),
            "local_filter": localFilter ?? NSNull(),
            "receipts": receipts ? "RECEIPTS" : NSNull(),
            "attested_bodies": attestedBodies ? "BODIES" : NSNull(),
            "session": [
                "heading": "H", "before_label": "B", "before_line": "BL",
                "after_label": "AF", "after_line": "AL",
            ],
        ]
        let data = try! JSONSerialization.data(withJSONObject: ["facts": facts, "copy": copy])
        return String(decoding: data, as: UTF8.self)
    }

    func testTheWitnessRouteDecodesWithItsPinsAndOrigin() throws {
        let value = try XCTUnwrap(RouteDisclosure.decode(fromJSON: payload()))
        XCTAssertTrue(value.sendsToWitness)
        XCTAssertEqual(value.facts.witness?.pinnedMeasurements, ["mrtd=aa"])
        XCTAssertEqual(value.copy.witness?.origin, "ORIGIN")
        XCTAssertEqual(value.copy.witness?.classifier, "CLASSIFIER")
    }

    func testTheLocalRouteCarriesItsFilterAndNoWitness() throws {
        let value = try XCTUnwrap(RouteDisclosure.decode(fromJSON: payload(
            route: "local", witness: false, localFilter: "FILTER", receipts: false)))
        XCTAssertEqual(value.copy.localFilter, "FILTER")
        XCTAssertNil(value.copy.witness)
    }

    /// Words for a block the facts do not have are refused, not rendered.
    func testMismatchedWordsAreRefused() {
        // A local route with a raw-send receipt line.
        XCTAssertNil(RouteDisclosure.decode(fromJSON: payload(
            route: "local", witness: false, localFilter: "FILTER", receipts: true)))
        // A refusing witness is sent nothing: no classifier line.
        XCTAssertNil(RouteDisclosure.decode(fromJSON: payload(
            route: "witness_refusing", classifier: true, receipts: false)))
        XCTAssertNotNil(RouteDisclosure.decode(fromJSON: payload(
            route: "witness_refusing", classifier: false, receipts: false)))
        // The witness route with no filter line but one supplied.
        XCTAssertNil(RouteDisclosure.decode(fromJSON: payload(localFilter: "FILTER")))
        XCTAssertNil(RouteDisclosure.decode(fromJSON: "not json"))
    }

    func testACertificateIsWordedOnlyForTheVerificationItNames() {
        let copy = #"{"heading":"H","measurement_label":"M","signer_label":"S","verified_at_review":"V"}"#
        let held = #"{"state":"held","verification":"verified_at_review","witness_measurement":"mrtd=aa","signer":"0xab"}"#
        let detail = CertificateDetail.decode(detailJSON: held, copyJSON: copy)
        XCTAssertEqual(detail?.witnessMeasurement, "mrtd=aa")
        XCTAssertEqual(detail?.verifiedAtReview, "V")
        let other = held.replacingOccurrences(of: "verified_at_review", with: "verified_now")
        XCTAssertNil(CertificateDetail.decode(detailJSON: other, copyJSON: copy))
    }
}
