import TCShellCore
import XCTest

/// Decoding `list_projects`.
///
/// Every payload here is what the daemon actually emits, not a convenient
/// shape. Two of these cases failed before this file existed, and because a
/// single bad element fails the whole array, the effect was that the "What to
/// watch" screen never listed a project at all -- it rendered its empty state
/// on every machine and looked like a screen with nothing to show.
final class ProjectRowTests: XCTestCase {
    /// The app's decoder, matching `DaemonDecoding.decoder()`: chrono writes
    /// RFC 3339, optionally with fractional seconds.
    private func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        decoder.dateDecodingStrategy = .custom { d in
            let text = try d.singleValueContainer().decode(String.self)
            if let x = withFraction.date(from: text) { return x }
            if let x = plain.date(from: text) { return x }
            throw DecodingError.dataCorrupted(
                .init(codingPath: d.codingPath, debugDescription: "unparseable timestamp")
            )
        }
        return decoder
    }

    private func rows(_ json: String) throws -> [ProjectRow] {
        struct Wrapper: Decodable { let projects: [ProjectRow] }
        return try decoder().decode(Wrapper.self, from: Data(json.utf8)).projects
    }

    /// The default state of every project, and the one that used to fail:
    /// the daemon spells ask-first `notify_only`.
    func testAskFirstIsSpelledNotifyOnlyOnTheWire() throws {
        let decoded = try rows(#"""
        {"projects":[{"project_id":"p_a","project_label":"frob","mode":"notify_only","added_at":null,"configured":false}]}
        """#)
        XCTAssertEqual(decoded.count, 1)
        XCTAssertEqual(decoded[0].mode, .ask)
    }

    /// A project the daemon has seen but the contributor has not decided
    /// about carries a null timestamp. On first run that is every project.
    func testADiscoveredProjectHasNoAddedAt() throws {
        let decoded = try rows(#"""
        {"projects":[{"project_id":"p_a","project_label":"frob","mode":"notify_only","added_at":null,"configured":false}]}
        """#)
        XCTAssertNil(decoded[0].addedAt)
        XCTAssertFalse(decoded[0].configured)
    }

    func testAConfiguredProjectCarriesItsTimestamp() throws {
        let decoded = try rows(#"""
        {"projects":[{"project_id":"p_a","project_label":"frob","mode":"ignore","added_at":"2026-08-19T10:00:00Z","configured":true}]}
        """#)
        XCTAssertNotNil(decoded[0].addedAt)
        XCTAssertTrue(decoded[0].configured)
        XCTAssertEqual(decoded[0].mode, .ignore)
    }

    /// The whole array must survive a mix. A single undecodable element
    /// throws for the entire response, which is how one null hid every
    /// project on the screen.
    func testAMixedListDecodesEntirely() throws {
        let decoded = try rows(#"""
        {"projects":[
          {"project_id":"p_a","project_label":"frob","mode":"notify_only","added_at":null,"configured":false},
          {"project_id":"p_b","project_label":"widget","mode":"ignore","added_at":"2026-08-19T10:00:00Z","configured":true},
          {"project_id":"p_c","project_label":"thing","mode":"auto_upload","added_at":"2026-08-19T10:00:00.123Z","configured":true}
        ]}
        """#)
        XCTAssertEqual(decoded.count, 3)
        XCTAssertEqual(decoded.map(\.mode), [.ask, .ignore, .autoUpload])
    }

    /// Identity is the id, not the label: two projects can share a final
    /// path segment, and a list keyed by label would collapse them.
    func testIdentityIsTheIdNotTheLabel() throws {
        let decoded = try rows(#"""
        {"projects":[
          {"project_id":"p_a","project_label":"api","mode":"notify_only","added_at":null,"configured":false},
          {"project_id":"p_b","project_label":"api","mode":"notify_only","added_at":null,"configured":false}
        ]}
        """#)
        XCTAssertEqual(Set(decoded.map(\.id)).count, 2, "two rows must stay two rows")
    }

    /// The bucket is whatever the daemon says it is -- never the row whose
    /// label happens to read like one. A contributor really can have a
    /// directory called `unknown-project`.
    func testTheBucketIsTakenFromTheDaemonNotTheLabel() throws {
        let decoded = try rows(#"""
        {"projects":[
          {"project_id":"p_a","project_label":"unknown-project","mode":"notify_only","added_at":null,"configured":false,"is_unresolved_bucket":false},
          {"project_id":"p_b","project_label":"Sessions","mode":"notify_only","added_at":null,"configured":false,"is_unresolved_bucket":true}
        ]}
        """#)
        XCTAssertFalse(
            decoded[0].isUnresolvedBucket,
            "a real project named unknown-project is still a real project"
        )
        XCTAssertTrue(decoded[1].isUnresolvedBucket)
    }

    /// A daemon predating the flag leaves rows plain rather than explained.
    /// Defaulting the other way would tell a contributor their own repository
    /// can never be contributed automatically, which is a lie about their
    /// project rather than a missing note.
    func testAMissingFlagMeansOrdinaryProject() throws {
        let decoded = try rows(#"""
        {"projects":[{"project_id":"p_a","project_label":"frob","mode":"notify_only","added_at":null,"configured":false}]}
        """#)
        XCTAssertFalse(decoded[0].isUnresolvedBucket)
    }
}
