import Foundation

/// One project's slice of the waiting queue: the entries themselves, plus
/// the totals the group header states.
///
/// The entries live *in* the group rather than being filtered back out of
/// the flat list by the view. That is the whole reason this type exists --
/// see `QueueGrouping`.
public struct QueueGroup<Entry>: Identifiable {
    /// `project_id`, and the id `submitProject` takes. Never the label: a
    /// label is a display name, is not guaranteed unique across two
    /// different projects, and grouping on it would merge them into one
    /// bucket with one `Submit all` that could approve the wrong project's
    /// entries.
    public let id: String
    /// The label the first entry of this project carried. A later entry
    /// with a stale label does not rename the group out from under its
    /// buttons.
    public let label: String
    /// Sum of the group's entry sizes.
    public let bytes: Int
    public let entries: [Entry]

    public var count: Int { entries.count }

    public init(id: String, label: String, bytes: Int, entries: [Entry]) {
        self.id = id
        self.label = label
        self.bytes = bytes
        self.entries = entries
    }
}

extension QueueGroup: Equatable where Entry: Equatable {}

/// Groups the waiting queue by project in a single pass.
///
/// This replaces a shape that cost entries times projects on *every*
/// SwiftUI body evaluation: the view grouped the waiting list to get the
/// project headers, and then, once per header, filtered the whole waiting
/// list again to find that project's rows. At the 500-entry cap the queue
/// actually runs at, with a dozen projects, that was tens of thousands of
/// entry visits and a dozen freshly allocated arrays per redraw -- and a
/// fresh array every time also denies SwiftUI any chance of deciding a
/// group had not changed.
///
/// Grouping once, off the model rather than out of the view, makes that
/// cost proportional to what actually moved.
public enum QueueGrouping {
    public static func groups<Entry>(
        _ entries: [Entry],
        projectID: (Entry) -> String,
        projectLabel: (Entry) -> String,
        sizeBytes: (Entry) -> Int
    ) -> [QueueGroup<Entry>] {
        var order: [String] = []
        var labels: [String: String] = [:]
        var bytes: [String: Int] = [:]
        var members: [String: [Entry]] = [:]

        for entry in entries {
            let id = projectID(entry)
            if members[id] == nil {
                order.append(id)
                labels[id] = projectLabel(entry)
                bytes[id] = 0
                members[id] = []
            }
            bytes[id]! += sizeBytes(entry)
            members[id]!.append(entry)
        }

        return order.map { id in
            QueueGroup(
                id: id,
                label: labels[id] ?? "",
                bytes: bytes[id] ?? 0,
                entries: members[id] ?? []
            )
        }
    }
}
