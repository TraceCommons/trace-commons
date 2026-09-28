import TCShellCore
import XCTest

/// K11's decoder, against stand-in words: which blocks are present is what
/// matters here, not what they say.
final class RouteDisclosureTests: XCTestCase {
    private func payload(
        route: String = "witness",
        witness: Bool = true,
        witnessWords: Bool? = nil,
        classifier: Bool = true,
        localFilter: String? = nil,
        receipts: Bool = true,
        attestedBodies: Bool = false,
        attestedBodiesWords: Bool? = nil
    ) -> String {
        let witnessWords = witnessWords ?? witness
        let attestedBodiesWords = attestedBodiesWords ?? attestedBodies
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
            "witness": witnessWords
                ? [
                    "heading": "W", "address_label": "A", "signing_label": "S",
                    "measurements_label": "M", "check": "CHECK",
                    "classifier": classifier ? "CLASSIFIER" : NSNull(), "origin": "ORIGIN",
                ] as [String: Any]
                : NSNull(),
            "local_filter": localFilter ?? NSNull(),
            "receipts": receipts ? "RECEIPTS" : NSNull(),
            "attested_bodies": attestedBodiesWords ? "BODIES" : NSNull(),
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

    /// The attested-bodies line is said exactly when the facts say prompts
    /// and replies go to the witness: on the witness route with the setting
    /// on, and nowhere else.
    func testMismatchedAttestedBodiesWordsAreRefused() {
        XCTAssertNotNil(RouteDisclosure.decode(fromJSON: payload(attestedBodies: true)))
        // The setting is on, but the words for it are missing.
        XCTAssertNil(RouteDisclosure.decode(fromJSON: payload(
            attestedBodies: true, attestedBodiesWords: false)))
        // The setting is off, but the words claim it is on.
        XCTAssertNil(RouteDisclosure.decode(fromJSON: payload(
            attestedBodies: false, attestedBodiesWords: true)))
        // A refusing witness is sent nothing, whatever the setting says.
        XCTAssertNil(RouteDisclosure.decode(fromJSON: payload(
            route: "witness_refusing", classifier: false, receipts: false,
            attestedBodies: true, attestedBodiesWords: true)))
        XCTAssertNotNil(RouteDisclosure.decode(fromJSON: payload(
            route: "witness_refusing", classifier: false, receipts: false,
            attestedBodies: true, attestedBodiesWords: false)))
    }

    /// Words for a witness the facts do not name are refused, and so is a
    /// named witness with no words for it.
    func testWitnessWordsWithoutWitnessFactsAreRefused() {
        XCTAssertNil(RouteDisclosure.decode(fromJSON: payload(
            route: "local", witness: false, witnessWords: true, classifier: false,
            localFilter: "FILTER", receipts: false)))
        XCTAssertNil(RouteDisclosure.decode(fromJSON: payload(witness: false, witnessWords: true)))
        XCTAssertNil(RouteDisclosure.decode(fromJSON: payload(witness: true, witnessWords: false)))
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
