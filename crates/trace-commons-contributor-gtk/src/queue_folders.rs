//! Grouping the waiting queue into folders, and remembering which one is
//! open.
//!
//! Both halves used to be absent: `render` grouped inline and there was only
//! one level to be on. The drill-in adds a second level that can be pulled
//! out from under the person standing on it -- approving a folder's last
//! session removes the folder, and so does an upload finishing in the
//! background.

use crate::model::QueueEntry;

/// One project's slice of the waiting queue.
pub struct Folder {
    /// `project_id`, and the id `submit_project` acts on. Never the label:
    /// a label is a display name, is not unique across two projects, and
    /// grouping on it would put one `Submit all` over another project's
    /// sessions.
    pub project_id: String,
    pub project_label: String,
    /// Taken from the first member. Every member of one project reports the
    /// same path, and a later entry with a stale one does not rename the
    /// folder out from under its buttons.
    pub project_path: String,
    pub bytes: u64,
    /// Each member paired with the index it had in the FLAT pending list.
    ///
    /// That index is load-bearing: `Look inside` opens the preview sheet by
    /// it, and the sheet re-derives its own copy of the pending list with an
    /// identical filter. Renumbering members inside a folder would open the
    /// wrong transcript.
    pub members: Vec<(usize, QueueEntry)>,
}

/// Which level of the queue is showing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Location {
    #[default]
    Root,
    /// One folder, by `project_id`.
    Project(String),
}

pub fn group(pending: &[&QueueEntry]) -> Vec<Folder> {
    let mut folders: Vec<Folder> = Vec::new();
    for (index, entry) in pending.iter().enumerate() {
        match folders
            .iter_mut()
            .find(|f| f.project_id == entry.project_id)
        {
            Some(folder) => {
                folder.bytes += entry.size_bytes;
                folder.members.push((index, (*entry).clone()));
            }
            None => folders.push(Folder {
                project_id: entry.project_id.clone(),
                project_label: entry.project_label.clone(),
                project_path: entry.project_path.clone(),
                bytes: entry.size_bytes,
                members: vec![(index, (*entry).clone())],
            }),
        }
    }
    folders
}

/// The location that is actually valid, given what the queue now holds.
///
/// A pure function of the location and the folders rather than a mutation,
/// so `render` can call it every time and never hold a stale location.
pub fn resolve(location: &Location, folders: &[Folder]) -> Location {
    match location {
        Location::Root => Location::Root,
        Location::Project(id) => {
            if folders.iter().any(|f| &f.project_id == id) {
                Location::Project(id.clone())
            } else {
                Location::Root
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, project: &str, label: &str, bytes: u64) -> QueueEntry {
        QueueEntry {
            entry_id: id.to_string(),
            project_id: project.to_string(),
            project_label: label.to_string(),
            project_path: format!("~/code/{label}"),
            size_bytes: bytes,
            state: "pending".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn an_empty_queue_has_no_folders() {
        assert!(group(&[]).is_empty());
    }

    #[test]
    fn folders_keep_first_seen_order() {
        let a = entry("1", "p2", "two", 1);
        let b = entry("2", "p1", "one", 1);
        let c = entry("3", "p2", "two", 1);
        let folders = group(&[&a, &b, &c]);
        assert_eq!(
            folders
                .iter()
                .map(|f| f.project_id.as_str())
                .collect::<Vec<_>>(),
            ["p2", "p1"]
        );
    }

    /// The index each entry had in the flat pending list must survive
    /// grouping: `Look inside` opens the preview sheet BY THAT INDEX, and
    /// the sheet re-derives its own copy of the pending list with the same
    /// filter. A folder that renumbered its members would open the wrong
    /// session's transcript.
    #[test]
    fn members_keep_their_flat_pending_index() {
        let a = entry("1", "p2", "two", 1);
        let b = entry("2", "p1", "one", 1);
        let c = entry("3", "p2", "two", 1);
        let folders = group(&[&a, &b, &c]);
        assert_eq!(
            folders[0]
                .members
                .iter()
                .map(|(i, _)| *i)
                .collect::<Vec<_>>(),
            [0, 2]
        );
        assert_eq!(
            folders[1]
                .members
                .iter()
                .map(|(i, _)| *i)
                .collect::<Vec<_>>(),
            [1]
        );
    }

    #[test]
    fn a_folder_sums_its_members_bytes() {
        let a = entry("1", "p1", "one", 30);
        let b = entry("2", "p1", "one", 12);
        assert_eq!(group(&[&a, &b])[0].bytes, 42);
    }

    #[test]
    fn a_folder_takes_the_path_of_its_first_member() {
        let a = entry("1", "p1", "one", 1);
        assert_eq!(group(&[&a])[0].project_path, "~/code/one");
    }

    #[test]
    fn two_projects_sharing_a_label_stay_separate() {
        let a = entry("1", "p1", "api", 1);
        let b = entry("2", "p2", "api", 1);
        assert_eq!(group(&[&a, &b]).len(), 2, "a label is not an identity");
    }

    #[test]
    fn root_stays_root() {
        assert!(matches!(resolve(&Location::Root, &[]), Location::Root));
    }

    #[test]
    fn a_folder_that_still_exists_is_kept() {
        let a = entry("1", "p1", "one", 1);
        let folders = group(&[&a]);
        assert!(matches!(
            resolve(&Location::Project("p1".into()), &folders),
            Location::Project(ref id) if id == "p1"
        ));
    }

    /// Submit all inside a folder empties it. Standing there would show a
    /// blank pane with a back button and no account of where it went.
    #[test]
    fn a_folder_that_emptied_falls_back_to_root() {
        let a = entry("1", "p2", "two", 1);
        let folders = group(&[&a]);
        assert!(matches!(
            resolve(&Location::Project("p1".into()), &folders),
            Location::Root
        ));
        assert!(matches!(
            resolve(&Location::Project("p1".into()), &[]),
            Location::Root
        ));
    }
}
