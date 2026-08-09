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

use std::collections::HashMap;

use zbus::zvariant::Value;

/// Ask the portal to acknowledge this application as a background app.
/// Fire-and-forget from the caller's point of view: spawns its own thread
/// and never blocks the caller, because a portal round trip includes
/// however long a permission dialog sits on screen, and startup must not
/// wait on that.
pub fn spawn_request() {
    std::thread::spawn(|| {
        if let Err(_error) = request_background() {
            // Fixed label only. No portal, an older portal without this
            // interface, and a contributor declining the dialog all reach
            // here, and none of them should look like a crash.
            eprintln!("trace-commons-shell: background portal unavailable");
        }
    });
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
