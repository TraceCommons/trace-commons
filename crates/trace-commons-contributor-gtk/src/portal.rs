//! `org.freedesktop.portal.Background`: where a GNOME user looks for
//! something that runs in the background, and the pause/quit surface for
//! anyone who never sees a tray.
//!
//! Not every desktop runs `xdg-desktop-portal`, and not every
//! `xdg-desktop-portal` that does run has a `Background` implementation
//! installed. Both are ordinary, not failures: this module is best-effort
//! and must never keep the application from starting or from being usable.
//! Every error from the call below is caught, logged with a fixed label
//! (never the D-Bus error text, which can carry more detail than a journal
//! line should), and dropped.
//!
//! `autostart: false` is deliberate. The portal's own `RequestBackground`
//! call *can* register an autostart entry on the caller's behalf, but this
//! application already has its own answer to "how does this start at
//! login" -- the systemd-unit-or-XDG-entry choice in `autostart.rs`, with
//! its own "never both" rule. Turning on the portal's autostart option too
//! would be a third mechanism, which is exactly the failure mode that rule
//! exists to prevent. (A Flatpak build is the one place this tradeoff cuts
//! the other way, because a confined app cannot write
//! `~/.config/autostart` itself -- see the report.)
//!
//! The call also doubles as a probe. A silent no-op on a desktop with no
//! `Background` backend is exactly the failure mode this product refuses
//! everywhere else, so `spawn_request` classifies its own outcome into a
//! [`BackendState`] and hands it back rather than only logging a fixed
//! line -- see `ui/settings.rs` for where a contributor is actually told.
//! It deliberately reuses the one real `RequestBackground` call already
//! being made rather than issuing a second one, so a backend-less desktop
//! is not asked twice and a backend that *does* exist never shows a second
//! permission dialog just to be probed.

use std::collections::HashMap;
use std::time::Duration;

use zbus::zvariant::Value;

/// Whether a `Background`-portal backend exists on this desktop, as
/// observed by asking rather than guessed from which desktop is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendState {
    /// Some implementation (GNOME Shell, Plasma, ...) fielded the call.
    /// Whether it went on to grant or deny persistent-background access is
    /// a separate question this probe does not answer.
    Present,
    /// The call failed with a "nothing here answers for that
    /// interface/method" D-Bus error: no backend implements `Background`
    /// at all. Verified against the real GTK and wlr portal backends,
    /// which reply `org.freedesktop.DBus.Error.UnknownMethod` /
    /// `UnknownInterface` for exactly this case; `ServiceUnknown` and
    /// `NameHasNoOwner` are the same "no such thing to answer this" family
    /// for the rarer case that `xdg-desktop-portal` itself is not running.
    Absent,
    /// The probe did not get a conclusive answer -- the session bus never
    /// replied within the bound below, or the failure was for a reason
    /// that says nothing about whether a backend exists (no session bus at
    /// all, for instance). Never treated as `Present` or `Absent`.
    Unknown,
}

/// How long the caller waits for a classification before moving on with
/// [`BackendState::Unknown`]. The underlying call may still be sitting on a
/// permission dialog past this point -- see `spawn_request`'s doc comment --
/// so this bounds only the caller's wait, not the request itself.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Ask the portal to acknowledge this application as a background app, and
/// report back whether any backend answered at all.
///
/// Fire-and-forget from the caller's point of view for the request itself:
/// spawns its own thread and never blocks the caller, because a portal
/// round trip includes however long a permission dialog sits on screen, and
/// startup must not wait on that. The classification is bounded separately
/// by [`PROBE_TIMEOUT`] so a UI reading the returned receiver never hangs
/// on a portal that never answers; if the dialog is still open past that
/// bound, the inner thread is simply left running (same as this module has
/// always done) and nothing further is ever sent.
pub fn spawn_request() -> async_channel::Receiver<BackendState> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let (inner_tx, inner_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let outcome = request_background();
            if outcome.is_err() {
                // Fixed label only. No portal, no `Background`
                // implementation, and a contributor declining the dialog
                // all reach here, and none of them should look like a
                // crash -- never the D-Bus error text, which can carry
                // more detail than a journal line should.
                eprintln!("trace-commons-shell: background portal unavailable");
            }
            let _ = inner_tx.send(classify(outcome));
        });
        let state = inner_rx
            .recv_timeout(PROBE_TIMEOUT)
            .unwrap_or(BackendState::Unknown);
        let _ = tx.send_blocking(state);
    });
    rx
}

/// Turn the outcome of the one real `RequestBackground` call into a
/// [`BackendState`]. `Ok` means some backend fielded the call; an error
/// with a D-Bus name from the "no such thing to answer this" family means
/// no backend implements `Background`; anything else is inconclusive.
fn classify(outcome: anyhow::Result<()>) -> BackendState {
    match outcome {
        Ok(()) => BackendState::Present,
        Err(error) => match error.downcast_ref::<zbus::Error>() {
            Some(zbus::Error::MethodError(name, _, _)) => {
                if is_absent_backend_error(name.as_str()) {
                    BackendState::Absent
                } else {
                    BackendState::Present
                }
            }
            _ => BackendState::Unknown,
        },
    }
}

/// The D-Bus error names that mean "nothing on the bus answers for this at
/// all", confirmed against a real `xdg-desktop-portal` with only the GTK
/// backend installed and no `Background` implementation available (see the
/// report for the exact transcript). Any other method-error reply means
/// some implementation received the call and answered it, even if that
/// answer was a decline.
fn is_absent_backend_error(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.UnknownMethod"
            | "org.freedesktop.DBus.Error.UnknownInterface"
            | "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
    )
}

fn request_background() -> anyhow::Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Background",
    )?;

    let handle_token = format!("tracecommons{}", std::process::id());
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(handle_token.as_str()));
    options.insert("reason", Value::from(crate::copy::PORTAL_BACKGROUND_REASON));
    options.insert("autostart", Value::from(false));

    let request_path: zbus::zvariant::OwnedObjectPath =
        proxy.call("RequestBackground", &("", options))?;

    // The call above only opens the request; the portal answers
    // asynchronously with a `Response` signal on the returned object. This
    // waits for it so the round trip is genuinely exercised end to end
    // rather than declared successful the moment the dialog opens -- but
    // the result of that wait changes nothing else here: the portal is the
    // permanent record of whether the contributor allowed it, and this
    // application has nothing further to persist about that decision.
    let request_proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        &request_path,
        "org.freedesktop.portal.Request",
    )?;
    let mut responses = request_proxy.receive_signal("Response")?;
    responses.next();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `zbus::Error::MethodError` carrying `name`, for exercising
    /// `classify` the same way it is exercised in production: through the
    /// actual error type, not a stand-in for it. The `Message` payload
    /// itself is never inspected by `classify`, so any well-formed one
    /// does -- it does not need to be an actual error reply.
    fn method_error(name: &str) -> anyhow::Error {
        let message =
            zbus::Message::method_call("/org/freedesktop/portal/desktop", "RequestBackground")
                .unwrap()
                .build(&())
                .unwrap();
        let error_name: zbus::names::OwnedErrorName =
            zbus::names::ErrorName::try_from(name).unwrap().into();
        anyhow::Error::new(zbus::Error::MethodError(error_name, None, message))
    }

    #[test]
    fn every_confirmed_no_backend_error_name_classifies_as_absent() {
        // UnknownMethod/UnknownInterface are what a real xdg-desktop-portal
        // with only the GTK backend installed actually returns for
        // RequestBackground (verified in a container -- see the report);
        // ServiceUnknown/NameHasNoOwner cover the portal daemon itself not
        // running at all.
        for name in [
            "org.freedesktop.DBus.Error.UnknownMethod",
            "org.freedesktop.DBus.Error.UnknownInterface",
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.NameHasNoOwner",
        ] {
            assert_eq!(
                classify(Err(method_error(name))),
                BackendState::Absent,
                "{name} should classify as no backend"
            );
        }
    }

    #[test]
    fn a_different_method_error_means_some_backend_fielded_it() {
        // A backend that exists but declines, or errors for some reason of
        // its own, still answered -- that is `Present`, not `Absent`.
        assert_eq!(
            classify(Err(method_error("org.freedesktop.portal.Error.NotAllowed"))),
            BackendState::Present
        );
    }

    #[test]
    fn a_successful_call_is_present() {
        assert_eq!(classify(Ok(())), BackendState::Present);
    }

    #[test]
    fn a_non_dbus_failure_is_unknown_rather_than_a_guess() {
        // Losing the session bus entirely, for instance, says nothing
        // about whether a Background backend exists -- it must not be
        // read as either answer.
        assert_eq!(
            classify(Err(anyhow::anyhow!("no session bus"))),
            BackendState::Unknown
        );
    }
}
