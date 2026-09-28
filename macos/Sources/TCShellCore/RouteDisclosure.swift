import Foundation

/// K11: what leaves this machine, to whom, and what this client checked.
///
/// `facts` is the daemon's `route_disclosure` answer; `copy` is the shared
/// crate's words for exactly those facts, both handed back by
/// `tc_route_disclosure_copy`. Nothing here is written in Swift, and the
/// decoder refuses -- rather than renders -- words for a block the facts do
/// not have: the ABI already sends only the blocks that are true of the
/// route, so a mismatch is a defect, and the safe rendering of a defect in a
/// privacy disclosure is none at all.
public struct RouteDisclosure: Decodable, Equatable, Sendable {
    public struct WitnessFacts: Decodable, Equatable, Sendable {
        public let state: String
        public let url: String
        public let signingAddress: String
        /// Verbatim, in stored order.
        public let pinnedMeasurements: [String]
        public let origin: String

        enum CodingKeys: String, CodingKey {
            case state, url, origin
            case signingAddress = "signing_address"
            case pinnedMeasurements = "pinned_measurements"
        }
    }

    public struct Facts: Decodable, Equatable, Sendable {
        /// `witness`, `witness_refusing`, `local`, `not_enrolled` or
        /// `settings_unreadable`.
        public let route: String
        public let witness: WitnessFacts?
        public let localFilter: String?
        public let attestedBodies: Bool

        enum CodingKeys: String, CodingKey {
            case route, witness
            case localFilter = "local_filter"
            case attestedBodies = "attested_bodies"
        }
    }

    public struct WitnessCopy: Decodable, Equatable, Sendable {
        public let heading: String
        public let addressLabel: String
        public let signingLabel: String
        public let measurementsLabel: String
        public let check: String
        /// The second enclave. Only on the witness route: a refusing witness
        /// is sent nothing, so nothing reaches a classifier.
        public let classifier: String?
        public let origin: String

        enum CodingKeys: String, CodingKey {
            case heading, check, classifier, origin
            case addressLabel = "address_label"
            case signingLabel = "signing_label"
            case measurementsLabel = "measurements_label"
        }
    }

    public struct SessionCopy: Decodable, Equatable, Sendable {
        public let heading: String
        public let beforeLabel: String
        public let beforeLine: String
        public let afterLabel: String
        public let afterLine: String

        enum CodingKeys: String, CodingKey {
            case heading
            case beforeLabel = "before_label"
            case beforeLine = "before_line"
            case afterLabel = "after_label"
            case afterLine = "after_line"
        }
    }

    public struct Copy: Decodable, Equatable, Sendable {
        public let title: String
        public let route: String
        public let witness: WitnessCopy?
        public let localFilter: String?
        public let receipts: String?
        public let attestedBodies: String?
        public let session: SessionCopy

        enum CodingKeys: String, CodingKey {
            case title, route, witness, receipts, session
            case localFilter = "local_filter"
            case attestedBodies = "attested_bodies"
        }
    }

    public let facts: Facts
    public let copy: Copy

    /// The `copy` fields this shell decodes, by wire name. Compared against
    /// the live export by `TCBridgeTests`, so a block added in Rust cannot be
    /// silently dropped here.
    public static let consumedCopyFields = [
        "title", "route", "witness", "local_filter", "receipts", "attested_bodies", "session",
    ]
    /// The same, for the nested `copy.witness` block -- where the
    /// both-enclaves and origin sentences live.
    public static let consumedWitnessCopyFields = [
        "heading", "address_label", "signing_label", "measurements_label", "check",
        "classifier", "origin",
    ]
    /// The same, for the nested `copy.session` block.
    public static let consumedSessionCopyFields = [
        "heading", "before_label", "before_line", "after_label", "after_line",
    ]

    /// Whether a session leaves this machine unredacted.
    public var sendsToWitness: Bool { facts.route == "witness" }

    /// Decode the ABI's payload, or nil if it will not parse, a sentence is
    /// empty, or its words do not match its facts: shown whole or not at all.
    public static func decode(fromJSON json: String) -> RouteDisclosure? {
        guard let data = json.data(using: .utf8),
            let value = try? JSONDecoder().decode(RouteDisclosure.self, from: data)
        else {
            return nil
        }
        return value.isConsistent ? value : nil
    }

    var isConsistent: Bool {
        let c = copy
        var lines = [c.title, c.route, c.session.heading, c.session.beforeLabel,
                     c.session.beforeLine, c.session.afterLabel, c.session.afterLine]
        if let w = c.witness {
            lines += [w.heading, w.addressLabel, w.signingLabel, w.measurementsLabel, w.check, w.origin]
        }
        if lines.contains(where: \.isEmpty) { return false }
        let optional = [c.localFilter, c.receipts, c.attestedBodies, c.witness?.classifier]
        if optional.contains(where: { $0?.isEmpty == true }) { return false }
        if (facts.witness == nil) != (c.witness == nil) { return false }
        if let w = c.witness, (w.classifier != nil) != sendsToWitness { return false }
        if (c.localFilter != nil) != (facts.route == "local") { return false }
        if (c.receipts != nil) != sendsToWitness { return false }
        if (c.attestedBodies != nil) != (sendsToWitness && facts.attestedBodies) { return false }
        return true
    }
}

/// What a disclosure surface says when the daemon's answer could not be
/// read (`tc_route_disclosure_unreadable_copy`). Said rather than left
/// blank, so a missing panel is not mistaken for nothing to disclose.
public struct RouteDisclosureUnreadable: Decodable, Equatable, Sendable {
    public let panel: String
    public let session: String

    public static let consumedFields = ["panel", "session"]

    public static func decode(fromJSON json: String) -> RouteDisclosureUnreadable? {
        guard let data = json.data(using: .utf8),
            let value = try? JSONDecoder().decode(RouteDisclosureUnreadable.self, from: data),
            !value.panel.isEmpty, !value.session.isEmpty
        else {
            return nil
        }
        return value
    }
}

/// The certificate a pending entry holds, as the daemon's
/// `certificate_detail` reports it, with the shared crate's labels
/// (`tc_certificate_detail_copy`).
public struct CertificateDetail: Equatable, Sendable {
    public let witnessMeasurement: String
    public let signer: String
    public let heading: String
    public let measurementLabel: String
    public let signerLabel: String
    /// The sentence for `verification: verified_at_review`, the only
    /// verification the daemon reports. Any other is not worded as this one.
    public let verifiedAtReview: String

    /// The label fields this shell decodes, compared against the export.
    public static let consumedCopyFields = [
        "heading", "measurement_label", "signer_label", "verified_at_review",
    ]

    public static func decode(detailJSON: String, copyJSON: String) -> CertificateDetail? {
        guard let detail = object(detailJSON), let copy = object(copyJSON),
            detail["verification"] as? String == "verified_at_review",
            let measurement = text(detail["witness_measurement"]),
            let signer = text(detail["signer"]),
            let heading = text(copy["heading"]),
            let measurementLabel = text(copy["measurement_label"]),
            let signerLabel = text(copy["signer_label"]),
            let verified = text(copy["verified_at_review"])
        else {
            return nil
        }
        return CertificateDetail(
            witnessMeasurement: measurement, signer: signer, heading: heading,
            measurementLabel: measurementLabel, signerLabel: signerLabel,
            verifiedAtReview: verified)
    }

    private static func object(_ json: String) -> [String: Any]? {
        guard let data = json.data(using: .utf8) else { return nil }
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
    }

    private static func text(_ value: Any?) -> String? {
        guard let string = value as? String, !string.isEmpty else { return nil }
        return string
    }
}
