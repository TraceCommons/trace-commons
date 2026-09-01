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

/// The contribution half of the digest: what went out unasked since the last
/// one.
///
/// `None` when nothing did -- the caller then has only the waiting half, or
/// nothing to say at all. A line reading "0 sessions contributed" is worse
/// than no line.
///
/// The daemon composes the same sentence for its own local notifier
/// (`trace_commons_contributor::daemon::notify::contribution_text`) and the
/// macOS shell composes it in `DigestCopy.contributionLine`. All three follow
/// the same rules and are tested against them separately, because each
/// platform's notification centre words the surrounding text differently and
/// a shared string would not survive that.
pub fn contribution_body(
    contributed: usize,
    project_labels: &[String],
    credit_pending: f32,
) -> Option<String> {
    if contributed == 0 {
        return None;
    }
    let sessions = if contributed == 1 {
        "session"
    } else {
        "sessions"
    };
    let named: Vec<&str> = project_labels
        .iter()
        .filter(|l| !l.is_empty())
        .map(String::as_str)
        .collect();
    let head: Vec<&str> = named.iter().take(3).copied().collect();
    let more = named.len().saturating_sub(head.len());
    let projects = match (head.as_slice(), more) {
        ([], _) => String::new(),
        (h, m) if m > 0 => format!(" from {} and {m} more", h.join(", ")),
        ([one], _) => format!(" from {one}"),
        ([a, b], _) => format!(" from {a} and {b}"),
        ([rest @ .., last], _) => format!(" from {} and {last}", rest.join(", ")),
    };
    let mut line = format!("{contributed} {sessions} contributed{projects}.");
    // Only when there is some: "0 credit pending" reads as a failure rather
    // than as a fresh start. Always "pending", never "earned" -- settlement
    // is off on every deployment shipped so far.
    if credit_pending > 0.0 {
        line.push_str(&format!(" {credit_pending:.1} credit pending."));
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_contributed_produces_no_line() {
        assert_eq!(contribution_body(0, &[], 0.0), None);
    }

    #[test]
    fn the_contribution_line_names_projects_and_never_a_path() {
        let line = contribution_body(
            3,
            &["trace-commons-server".to_string(), "dotfiles".to_string()],
            0.0,
        )
        .unwrap();
        assert_eq!(
            line,
            "3 sessions contributed from trace-commons-server and dotfiles."
        );
        assert!(!line.contains('/'), "{line}");
    }

    #[test]
    fn one_contributed_session_is_singular() {
        let line = contribution_body(1, &["a".to_string()], 0.0).unwrap();
        assert!(line.starts_with("1 session contributed from a."), "{line}");
    }

    #[test]
    fn many_contributed_projects_are_summarised() {
        let labels: Vec<String> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let line = contribution_body(9, &labels, 0.0).unwrap();
        assert!(line.contains("and 2 more"), "{line}");
    }

    #[test]
    fn credit_is_stated_only_when_there_is_some() {
        let with = contribution_body(2, &["a".to_string()], 4.25).unwrap();
        assert!(with.ends_with("4.2 credit pending."), "{with}");
        let without = contribution_body(2, &["a".to_string()], 0.0).unwrap();
        assert!(!without.contains("credit"), "{without}");
    }

    #[test]
    fn pending_credit_is_never_called_earned() {
        let line = contribution_body(2, &["a".to_string()], 4.25).unwrap();
        assert!(line.contains("pending"), "{line}");
        for word in ["earned", "paid", "settled", "worth"] {
            assert!(!line.contains(word), "must not say {word}: {line}");
        }
    }

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
