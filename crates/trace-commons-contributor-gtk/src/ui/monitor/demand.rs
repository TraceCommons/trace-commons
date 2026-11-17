//! The rule that opens the inspector on demand.
//!
//! A port of `InspectorDemand` in
//! `macos/Sources/TraceCommonsApp/Views/Monitor/MonitorShell.swift` and its
//! `keys(model:traces:selection:)` in `TracesInspectorHost.swift`. The
//! inspector starts closed in a narrow window, but the things a person must
//! not miss open it when they appear. Something going away, or the same
//! thing still asking, leaves the person's choice. Owner-approved for this
//! shell on 2026-11-16 (spec, "Owner decisions", 1).

use std::collections::BTreeSet;

/// One key per thing asking for the inspector right now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asking<'a> {
    /// The entries an approval's undo window covers.
    pub approval_undo: Option<&'a [String]>,
    /// When that approval was made, so a second approval of the same
    /// session is a new undo and a tick of its clock is not.
    pub approval_at: Option<i64>,
    pub contributed: Option<&'a str>,
    pub kept: Option<&'a str>,
    pub contributed_folder: Option<&'a str>,
    pub submitting_folder: Option<&'a str>,
    /// A selected session: its card lives in the inspector. A folder is not
    /// a demand.
    pub review: Option<&'a str>,
}

pub fn keys(asking: &Asking<'_>) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    if let Some(ids) = asking.approval_undo {
        keys.insert(format!("undo:{}", ids.join(",")));
    }
    if let Some(at) = asking.approval_at {
        keys.insert(format!("undo:approval-at:{at}"));
    }
    if let Some(id) = asking.contributed {
        keys.insert(format!("contributed:{id}"));
    }
    if let Some(id) = asking.kept {
        keys.insert(format!("kept:{id}"));
    }
    if let Some(id) = asking.contributed_folder {
        keys.insert(format!("folder-undo:{id}"));
    }
    if let Some(id) = asking.submitting_folder {
        keys.insert(format!("folder-submit:{id}"));
    }
    if let Some(id) = asking.review {
        keys.insert(format!("review:{id}"));
    }
    keys
}

/// The inspector opens when something new asks for it.
pub fn opens(previous: &BTreeSet<String>, current: &BTreeSet<String>) -> bool {
    current.difference(previous).next().is_some()
}

/// Whether the keys there when the window appears open the inspector: any
/// at all, so a closed inspector restored from the last run does not hide
/// an undo made while the window was closed.
pub fn opens_on_appear(keys: &BTreeSet<String>) -> bool {
    opens(&BTreeSet::new(), keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_asking_is_no_key() {
        assert!(keys(&Asking::default()).is_empty());
        assert!(!opens_on_appear(&keys(&Asking::default())));
    }

    #[test]
    fn something_new_opens_the_inspector() {
        let none = keys(&Asking::default());
        let submitting = keys(&Asking {
            submitting_folder: Some("p1"),
            ..Asking::default()
        });
        assert!(opens(&none, &submitting));
        let undo = keys(&Asking {
            contributed_folder: Some("p1"),
            ..Asking::default()
        });
        assert!(opens(&submitting, &undo), "a folder's undo is a new key");
        let ids = ["e1".to_string()];
        let more = keys(&Asking {
            approval_undo: Some(&ids),
            contributed: Some("e1"),
            kept: Some("e2"),
            ..Asking::default()
        });
        assert_eq!(more.len(), 3);
        assert!(opens(&undo, &more));
    }

    #[test]
    fn the_same_thing_still_asking_or_something_going_away_opens_nothing() {
        let ids = ["e1".to_string(), "e2".to_string()];
        let both = keys(&Asking {
            approval_undo: Some(&ids),
            review: Some("e9"),
            ..Asking::default()
        });
        assert!(!opens(&both, &both));
        let fewer = keys(&Asking {
            review: Some("e9"),
            ..Asking::default()
        });
        assert!(!opens(&both, &fewer), "a key going away closes nothing");
        assert!(!opens(&both, &keys(&Asking::default())));
    }

    #[test]
    fn a_second_approval_of_the_same_session_is_a_new_undo() {
        let ids = ["e1".to_string()];
        let first = keys(&Asking {
            approval_undo: Some(&ids),
            approval_at: Some(100),
            ..Asking::default()
        });
        let again = keys(&Asking {
            approval_undo: Some(&ids),
            approval_at: Some(160),
            ..Asking::default()
        });
        assert!(opens(&first, &again));
        assert!(!opens(&first, &first), "a tick of the hold is not");
    }

    #[test]
    fn selecting_a_session_asks_and_a_folder_does_not() {
        let session = keys(&Asking {
            review: Some("e1"),
            ..Asking::default()
        });
        assert_eq!(session.into_iter().collect::<Vec<_>>(), ["review:e1"]);
        // A folder selection is carried by no field at all.
        assert!(keys(&Asking::default()).is_empty());
    }

    #[test]
    fn a_demand_already_there_when_the_window_appears_opens_it() {
        let ids = ["e1".to_string()];
        assert!(opens_on_appear(&keys(&Asking {
            approval_undo: Some(&ids),
            ..Asking::default()
        })));
    }
}
