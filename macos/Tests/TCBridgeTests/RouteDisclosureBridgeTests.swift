import TCBridge
import TCShellCore
import XCTest

/// K11 through the real dylib: the daemon's facts go in, the Rust's words
/// come out, and this shell decodes all of them.
final class RouteDisclosureBridgeTests: XCTestCase {
    private let witnessFacts = """
        {"route":"witness","witness":{"state":"pinned","url":"https://witness.example",\
        "signing_address":"0xab","pinned_measurements":["mrtd=aa"],"origin":"published_at_join"},\
        "local_filter":null,"receipts":{"endpoint_configured":true,"check_attestation":false},\
        "attested_bodies":false}
        """

    func testTheWitnessRouteBecomesTheRustsDisclosure() throws {
        let json = try XCTUnwrap(TCConsentCopy.routeDisclosureJSON(forFacts: witnessFacts))
        let value = try XCTUnwrap(RouteDisclosure.decode(fromJSON: json))
        XCTAssertTrue(value.sendsToWitness)
        XCTAssertEqual(value.facts.witness?.origin, "published_at_join")
        XCTAssertTrue(value.copy.witness?.origin.contains("without asking you") == true)
        XCTAssertTrue(value.copy.receipts?.contains("tells the provider") == true)
        XCTAssertNil(value.copy.localFilter)
    }

    func testTheLocalRouteBecomesTheRustsDisclosure() throws {
        let facts = """
            {"route":"local","witness":null,"local_filter":"near_ai",\
            "receipts":{"endpoint_configured":false,"check_attestation":false},"attested_bodies":false}
            """
        let json = try XCTUnwrap(TCConsentCopy.routeDisclosureJSON(forFacts: facts))
        let value = try XCTUnwrap(RouteDisclosure.decode(fromJSON: json))
        XCTAssertNil(value.copy.witness)
        XCTAssertTrue(value.copy.localFilter?.contains("NEAR AI") == true)
    }

    /// A route or origin newer than this build is not rendered as the
    /// nearest one: the ABI answers nil.
    func testAnUnknownRouteOrOriginIsNil() {
        XCTAssertNil(TCConsentCopy.routeDisclosureJSON(forFacts: #"{"route":"somewhere_new"}"#))
        let unknownOrigin = witnessFacts.replacingOccurrences(
            of: "published_at_join", with: "an_operator")
        XCTAssertNil(TCConsentCopy.routeDisclosureJSON(forFacts: unknownOrigin))
    }

    /// The exported field sets are exactly what this shell decodes.
    func testTheExportedFieldsAreExactlyTheOnesThisShellConsumes() throws {
        let json = try XCTUnwrap(TCConsentCopy.routeDisclosureJSON(forFacts: witnessFacts))
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        let copy = try XCTUnwrap(object["copy"] as? [String: Any])
        XCTAssertEqual(copy.keys.sorted(), RouteDisclosure.consumedCopyFields.sorted())
        // The nested blocks too: the both-enclaves and origin sentences live
        // in `witness`, and a sentence added there in Rust must not be
        // dropped silently here. The witness route sends every witness key.
        let witness = try XCTUnwrap(copy["witness"] as? [String: Any])
        XCTAssertEqual(witness.keys.sorted(), RouteDisclosure.consumedWitnessCopyFields.sorted())
        let session = try XCTUnwrap(copy["session"] as? [String: Any])
        XCTAssertEqual(session.keys.sorted(), RouteDisclosure.consumedSessionCopyFields.sorted())
        let labels = try XCTUnwrap(TCConsentCopy.certificateDetailCopyJSON())
        let labelObject = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(labels.utf8)) as? [String: Any])
        XCTAssertEqual(labelObject.keys.sorted(), CertificateDetail.consumedCopyFields.sorted())
        let unreadable = try XCTUnwrap(TCConsentCopy.routeDisclosureUnreadableJSON())
        let unreadableObject = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(unreadable.utf8)) as? [String: Any])
        XCTAssertEqual(
            unreadableObject.keys.sorted(), RouteDisclosureUnreadable.consumedFields.sorted())
        XCTAssertNotNil(RouteDisclosureUnreadable.decode(fromJSON: unreadable))
    }
}
