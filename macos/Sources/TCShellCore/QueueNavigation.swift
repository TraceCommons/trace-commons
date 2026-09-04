import Foundation

/// Which level of the queue is on screen.
public enum QueueLocation: Equatable, Hashable {
    /// The folder list.
    case root
    /// One folder's sessions, by `project_id` -- never by label, which two
    /// different projects can share.
    case project(String)
}

/// Keeping the queue's location honest as the queue moves.
///
/// The drill-in level names a project that may stop existing at any moment:
/// approving a folder's last session removes the folder, and so does an
/// upload finishing in the background. Without this, the detail view would
/// be left rendering an empty list with a back button and no account of
/// where its contents went.
///
/// This is a pure function of the location and the current groups rather
/// than a mutation, so the view can call it on every redraw and the
/// resolved location is never stale.
public enum QueueNavigation {
    /// The location that is actually valid, given what the queue now holds.
    /// A project that is gone resolves to `.root`.
    public static func resolve<Entry>(
        _ location: QueueLocation,
        in groups: [QueueGroup<Entry>]
    ) -> QueueLocation {
        switch location {
        case .root:
            return .root
        case .project(let id):
            return groups.contains(where: { $0.id == id }) ? .project(id) : .root
        }
    }
}
