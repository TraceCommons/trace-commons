//! The words for a watcher that is already running, in one place, for all
//! three shells.
//!
//! `tc_daemon_start` reports `already-running` when another process holds the
//! state directory's lock, and the C ABI documents that label as "not an
//! error to repair: the daemon the contributor wants is already up". A shell
//! acts on it by attaching (`tc_daemon_attach`), which leaves it driving a
//! watcher it does not own.
//!
//! Two things that window cannot do, and both are worth a sentence rather
//! than a dead control: it cannot stop the watcher, and it cannot open a
//! trace's redacted body -- the socket carries the summary only.
//!
//! # What crosses the boundary
//!
//! The same contract [`crate::routing_copy`] states: the sentences cross
//! already assembled, so a shell renders them and does not compose them.
//! These exist here rather than in Swift because a sentence written in one
//! shell is one the other two never get, and one a rewording here would
//! never reach.
//!
//! Nothing here names a path, a process id, or which application holds the
//! lock. The shell asking cannot know whether the holder is another copy of
//! itself or a service-managed daemon, and a sentence that guessed would be
//! wrong for whichever case it guessed against.

use serde::Serialize;

/// Every fixed word on the attached-watcher surface.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AttachCopy {
    /// The banner's heading.
    pub attached_title: &'static str,
    /// What this window can and cannot do while attached.
    pub attached_detail: &'static str,
    /// Said when a start is refused because a watcher is already running and
    /// the shell has not (or cannot) attach.
    pub already_running: &'static str,
    /// Said when an attach finds nothing listening.
    pub no_daemon_listening: &'static str,
    /// The state directory itself could not be opened or written.
    pub state_directory_not_writable: &'static str,
    /// `daemon-settings.json` cannot be parsed by this version.
    pub settings_unreadable: &'static str,
    /// The control socket could not be bound.
    pub ipc_bind_failed: &'static str,
}

/// The attached-watcher surface's words.
pub fn attach_copy() -> AttachCopy {
    AttachCopy {
        attached_title: "Another copy of this app is running the watcher.",
        attached_detail: "This window can review and approve. It can't stop the watcher or \
                          open a trace's full text. Quit the other copy and reopen this one \
                          for the whole app.",
        already_running: "A watcher is already running for this folder.",
        no_daemon_listening: "No watcher is listening for this folder.",
        state_directory_not_writable: "This app can't write to its own folder, so there's \
                                       nowhere to keep the queue. Nothing is being watched.",
        settings_unreadable: "This app's settings file can't be read by this version. \
                              Nothing is being watched.",
        ipc_bind_failed: "This app couldn't open its control socket. Nothing is being watched.",
    }
}

/// The sentence for a fixed start-failure label, or `None` for one this
/// surface does not own.
///
/// `None` rather than a fallback sentence on purpose: `trace_commons.h` says
/// to treat an unrecognized label as `daemon-start-failed` rather than
/// assuming it is safe to display, and a copy table that invented a sentence
/// for an unknown label would be doing exactly that.
pub fn line_for_label(label: &str) -> Option<&'static str> {
    let copy = attach_copy();
    match label {
        "already-running" => Some(copy.already_running),
        "no-daemon-listening" => Some(copy.no_daemon_listening),
        "state-directory-not-writable" => Some(copy.state_directory_not_writable),
        "settings-unreadable" => Some(copy.settings_unreadable),
        "ipc-bind-failed" => Some(copy.ipc_bind_failed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The labels this table answers are exactly the ones the ABI names, so
    /// a label gaining a sentence here and a label existing are the same
    /// question.
    #[test]
    fn every_named_start_failure_has_a_sentence() {
        for label in [
            "already-running",
            "no-daemon-listening",
            "state-directory-not-writable",
            "settings-unreadable",
            "ipc-bind-failed",
        ] {
            assert!(
                line_for_label(label).is_some(),
                "{label} has no sentence on this surface"
            );
        }
    }

    /// An unknown label gets nothing rather than a guess -- the header's own
    /// instruction.
    #[test]
    fn an_unknown_label_gets_no_sentence() {
        assert_eq!(line_for_label("daemon-start-failed"), None);
        assert_eq!(line_for_label("roots-not-declared"), None);
        assert_eq!(line_for_label(""), None);
    }

    /// Nothing here names a path, a process, or which app holds the lock.
    #[test]
    fn no_sentence_guesses_who_holds_the_lock() {
        let copy = attach_copy();
        for sentence in [
            copy.attached_title,
            copy.attached_detail,
            copy.already_running,
            copy.no_daemon_listening,
            copy.state_directory_not_writable,
            copy.settings_unreadable,
            copy.ipc_bind_failed,
        ] {
            assert!(
                !sentence.contains('/') && !sentence.contains("daemon.lock"),
                "a sentence on this surface names a path: {sentence}"
            );
        }
    }
}
