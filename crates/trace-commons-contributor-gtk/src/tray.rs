//! `StatusNotifierItem`: a bonus, never a foundation.
//!
//! GNOME has no tray without a user-installed shell extension, and the
//! whole point of this platform's design is that the app must not depend
//! on one. So this module has exactly one job when there is no
//! `StatusNotifierWatcher` on the session bus: fail quietly and change
//! nothing else. There is no code path anywhere in this application that
//! tells a contributor to install an extension to get this back -- the
//! window already does everything the tray would.
//!
//! Where a watcher *is* real (KDE, Cinnamon, XFCE, GNOME with the
//! extension), this exports a minimal `org.kde.StatusNotifierItem` object
//! and registers it. The icon does exactly one thing: a primary or
//! secondary click, or opening its context menu, all raise the window at
//! the queue. There is no menu vocabulary beyond that, deliberately
//! matching `notify.rs`'s rule that nothing reachable from outside the
//! window may approve or send anything.

use std::sync::Arc;

use zbus::interface;

/// What the tray icon does. `notify.rs` has the same shape for the same
/// reason: a surface reachable when the contributor is not looking at the
/// window must have the smallest possible vocabulary, and "open the
/// window" is the whole of it.
struct Item {
    tx: async_channel::Sender<()>,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        crate::ui::APP_ID
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        crate::copy::APP_NAME
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    /// Looked up by name from the icon theme, not a path -- the same rule
    /// the rest of this application holds for anything a contributor can
    /// see. A theme without this name falls back to no icon rather than a
    /// broken one; that degradation is acceptable for a bonus surface.
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        crate::ui::APP_ID
    }

    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(());
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(());
    }

    /// No menu is exported (`Menu` is deliberately absent from this
    /// object), so a host that asks for a context menu gets nothing to
    /// show. Raising the window in response is the same fallback the
    /// shared spec expects a tray-less desktop to reach anyway.
    fn context_menu(&self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(());
    }
}

/// Try to become a tray icon, in the background. Never blocks the caller
/// and never reports failure anywhere a contributor would see it: absence
/// of a watcher is the normal case on the majority desktop, not an error.
///
/// `on_activate` fires once per click/menu request, for as long as the
/// process lives. The receiving end (see `ui::App`) is what actually
/// raises the window; this module only produces the signal.
pub fn spawn() -> async_channel::Receiver<()> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        if let Err(_error) = register(tx) {
            // Fixed label. No watcher (plain GNOME, most Linux desktops
            // today) and a watcher that rejects the registration both land
            // here, and neither is a bug in this application.
            eprintln!("trace-commons-shell: tray unavailable");
        }
    });
    rx
}

fn register(tx: async_channel::Sender<()>) -> anyhow::Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    connection
        .object_server()
        .at("/StatusNotifierItem", Item { tx })?;

    let unique_name = connection
        .unique_name()
        .ok_or_else(|| anyhow::anyhow!("no-unique-bus-name"))?
        .to_string();

    let watcher = zbus::blocking::Proxy::new(
        &connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )?;
    watcher.call::<_, _, ()>("RegisterStatusNotifierItem", &(unique_name.as_str()))?;

    // The object server dispatches on zbus's own executor threads for as
    // long as `connection` lives; this thread's only remaining job is to
    // keep it alive, which an `Arc` kept in a park loop does without
    // spinning.
    let keepalive = Arc::new(connection);
    loop {
        std::thread::park();
        // `park` can return spuriously; the loop just keeps the `Arc` (and
        // therefore the connection) alive rather than trusting any one
        // wake-up to mean something.
        std::hint::black_box(&keepalive);
    }
}
