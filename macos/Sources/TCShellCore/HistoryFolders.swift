import Foundation

/// Grouping history records into folders, on the same `QueueGrouping` the
/// queue uses so the two screens navigate identically.
///
/// The wrinkle is records that predate the daemon carrying `project_id`,
/// and records submitted before project keys were normalized -- both arrive
/// with an empty id. Grouping on the empty string would sweep every one of
/// them into a single row spanning unrelated repositories.
///
/// So an unidentified record falls back to grouping by label, under a
/// synthetic key that cannot collide with a real `proj_` id. That merges two
/// same-named repositories, which is a real loss -- but it is the loss that
/// was already there before this screen grouped at all, and it is smaller
/// than merging everything. An unidentified record is never merged with an
/// identified one: same label or not, claiming they are the same folder is a
/// guess.
public enum HistoryFolders {
    /// The prefix for a group keyed by label because its records carry no
    /// project id. `project_id_for` always emits `proj_`, so these two key
    /// spaces cannot collide.
    public static let unresolvedPrefix = "label:"

    public static func folders<Record>(
        _ records: [Record],
        projectID: (Record) -> String,
        projectLabel: (Record) -> String
    ) -> [QueueGroup<Record>] {
        QueueGrouping.groups(
            records,
            projectID: { record in
                let id = projectID(record)
                return id.isEmpty ? unresolvedPrefix + projectLabel(record) : id
            },
            projectLabel: projectLabel,
            // History records carry no size: a submission's bytes are not
            // part of what `list_history` reports, and inventing a zero is
            // the only honest answer. The folder rows show a count only.
            sizeBytes: { _ in 0 }
        )
    }
}
