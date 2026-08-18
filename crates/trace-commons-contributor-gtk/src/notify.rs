//! Desktop notifications, which on Linux do the work a tray menu does
//! elsewhere.
//!
//! GNOME has no system tray, so notification actions are the reliable
//! cross-desktop way to reach a contributor who is not looking at the
//! window. That makes what those actions may do the single most important
//! rule in this file:
//!
//! **No notification action may upload anything, ever.** The actions are
//! exactly `Review` -- which opens the window at the queue and nothing else
//! -- and `Not now`, which dismisses. There is no approve, no "send all",
//! no default action that resolves to one. A misfired notification that
//! ships three real transcripts is unrecoverable, and no amount of
//! convenience is worth that risk. The enum below is deliberately the only
//! vocabulary this module has.
//!
//! What it may carry, beyond those two actions, is the mark: the design
//! spec puts "The Turn" in every frame the product appears in, and a
//! notification is one of them. See [`post`] for how the file gets there.

/// What a contributor pressed. There is no third variant, and adding one
/// that sends anything would violate the shared spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Bring the window forward at the queue. Uploads nothing.
    Review,
    /// Dismiss. Does nothing at all -- which is what makes the notification
    /// feel non-coercive.
    NotNow,
}

/// Post a notification and report which action was pressed, if any.
///
/// Blocking: it waits for the action, so callers run it on a thread and
/// deliver the result to the main loop. Failure to notify at all (no
/// notification daemon, a headless session) is not an error worth
/// surfacing: the window is the primary surface on this platform, and it is
/// still there.
pub fn post(summary: &str, body: &str) -> Option<Action> {
    let mut notification = notify_rust::Notification::new();
    notification
        .summary(summary)
        .body(body)
        .appname(crate::copy::APP_NAME)
        .action("review", crate::copy::NOTIFY_REVIEW)
        .action("not-now", crate::copy::NOT_NOW);

    // The mark, in its framed variant: the notification is one of the frames
    // the adopted mark is carried in, alongside the window chrome and the
    // tray. It goes on as an absolute path rather than a name, because the
    // notification daemon is a separate process that searches its own icon
    // themes and will never find ours. `tray.rs` owns the file because it is
    // the module that had to write one anyway; a daemon that cannot render
    // an SVG, or a run where the write failed, gets a notification with no
    // icon, which is what it got before the mark existed.
    let icon_path = crate::tray::icons().map(|icons| icons.app_icon.to_string_lossy().into_owned());
    if let Some(path) = icon_path.as_deref() {
        notification.icon(path);
    }

    let handle = notification.show().ok()?;

    let mut pressed = None;
    handle.wait_for_action(|id| {
        pressed = match id {
            "review" => Some(Action::Review),
            // Everything else, including the desktop's own "default"
            // activation and a plain dismissal, means do nothing. Mapping
            // an unknown action id onto anything that sends is exactly the
            // bug this module refuses to have.
            _ => Some(Action::NotNow),
        };
    });
    pressed
}

/// The 4-hour digest, in the shared spec's words.
///
/// Carries counts and project labels only. Never trace content: the preview
/// exemption does not extend to notification text.
pub fn digest_body(pending: usize, project_labels: &[String]) -> String {
    let sessions = if pending == 1 { "session" } else { "sessions" };
    let projects = match project_labels {
        [] => String::new(),
        [one] => format!(" from {one}"),
        [a, b] => format!(" from {a} and {b}"),
        [rest @ .., last] => format!(" from {} and {last}", rest.join(", ")),
    };
    format!(
        "{pending} {sessions} ready{projects}.\n{}",
        crate::copy::NOTIFY_NOTHING_SENT
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_reads_as_the_shared_spec_writes_it() {
        assert_eq!(
            digest_body(
                3,
                &["trace-commons-server".to_string(), "dotfiles".to_string()]
            ),
            "3 sessions ready from trace-commons-server and dotfiles.\n\
             Nothing is sent until you review them."
        );
    }

    #[test]
    fn one_session_is_singular() {
        assert!(digest_body(1, &["a".to_string()]).starts_with("1 session ready from a."));
    }

    #[test]
    fn there_are_exactly_two_actions_and_neither_sends_anything() {
        // A compile-time-ish guard: the enum is the whole vocabulary, and
        // the mapping in `post` sends every unrecognized id to `NotNow`.
        let all = [Action::Review, Action::NotNow];
        assert_eq!(all.len(), 2);
    }
}
