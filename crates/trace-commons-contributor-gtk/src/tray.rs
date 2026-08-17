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
//!
//! ## The icon
//!
//! The icon is "The Turn", in the status-bar template variant the design
//! spec defines for exactly this position: frameless, a single ink, stroke
//! 8/64 so the brackets survive the loss of the frame at 14 and 16 px.
//! `ui::mark` is the only description of that geometry in the application;
//! this module serialises it rather than restating it.
//!
//! The `StatusNotifierItem` protocol names an icon, it does not carry one,
//! so the mark has to exist as a file somewhere the host can read. This
//! module writes a small icon theme of its own -- two SVGs and an
//! `index.theme`, under the application's data directory -- and hands the
//! host its root in `IconThemePath`. That is the protocol's own mechanism
//! for an application whose icon is not installed system-wide, and it
//! keeps us out of `~/.local/share/icons`, which belongs to the packaging
//! and to the contributor, not to a running process.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use zbus::interface;

use crate::ui::mark;

/// What the tray icon does. `notify.rs` has the same shape for the same
/// reason: a surface reachable when the contributor is not looking at the
/// window must have the smallest possible vocabulary, and "open the
/// window" is the whole of it.
struct Item {
    tx: async_channel::Sender<()>,
    /// The name the host looks up, and the theme root it looks it up in.
    /// Both are resolved once, before this object exists, because a
    /// property getter runs on a zbus worker thread and may not touch GTK.
    icon_name: String,
    icon_theme_path: String,
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

    /// Looked up by name from an icon theme, not by path -- the same rule
    /// the rest of this application holds for anything a contributor can
    /// see. The name is the template variant of the mark, written by
    /// [`install_icons`] into the theme [`Self::icon_theme_path`] points
    /// at. If that write failed we fall back to the bare application id,
    /// which a system theme may or may not know: no icon is a worse
    /// outcome than the mark, but it is still better than a broken one,
    /// and this is a bonus surface.
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        &self.icon_name
    }

    /// Where the host should look for [`Self::icon_name`], in addition to
    /// the themes it already searches. Empty when we could not write the
    /// theme, which hosts read as "nothing extra to search".
    ///
    /// Hosts that ignore this property entirely (it is a KDE extension to
    /// the specification, not part of the original interface) fall back to
    /// searching their own themes for the name above, which is the same
    /// degradation as before the mark existed.
    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        &self.icon_theme_path
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
    // On the caller's thread, which is the main one: the ink of the mark
    // follows the desktop's light/dark preference, and `adw::StyleManager`
    // may only be read from the main thread. Everything after this point
    // runs on a thread that must never touch GTK, so the icons are written
    // -- and their paths frozen -- here.
    let icons = install_icons(mark::Scheme::current());
    let icon_name = icons
        .map(|icons| icons.status_name.clone())
        .unwrap_or_else(|| crate::ui::APP_ID.to_string());
    let icon_theme_path = icons
        .map(|icons| icons.theme_root.to_string_lossy().into_owned())
        .unwrap_or_default();

    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        if let Err(_error) = register(tx, icon_name, icon_theme_path) {
            // Fixed label. No watcher (plain GNOME, most Linux desktops
            // today) and a watcher that rejects the registration both land
            // here, and neither is a bug in this application.
            eprintln!("trace-commons-shell: tray unavailable");
        }
    });
    rx
}

fn register(
    tx: async_channel::Sender<()>,
    icon_name: String,
    icon_theme_path: String,
) -> anyhow::Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    connection.object_server().at(
        "/StatusNotifierItem",
        Item {
            tx,
            icon_name,
            icon_theme_path,
        },
    )?;

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

/// The mark, on disk, for the two surfaces outside the window that can only
/// take a named or a serialised icon.
///
/// Written once per process. `notify.rs` reads [`Icons::app_icon`] for the
/// notification's application icon; this module hands
/// [`Icons::status_name`] and [`Icons::theme_root`] to the tray host.
pub(crate) struct Icons {
    /// Root of the private icon theme: the directory that holds
    /// `index.theme`, and the one a tray host is given.
    pub(crate) theme_root: PathBuf,
    /// The framed variant, as an absolute path. A notification daemon takes
    /// a path directly and never searches our theme, so it gets this.
    pub(crate) app_icon: PathBuf,
    /// The template variant's name inside the theme, without a directory or
    /// an extension, which is the only form the tray protocol accepts.
    pub(crate) status_name: String,
}

static ICONS: OnceLock<Option<Icons>> = OnceLock::new();

/// Write the mark into a private icon theme, once, and remember the result.
///
/// `scheme` must be read on the main thread by the caller. Failure is not
/// reported anywhere a contributor would see it: a missing tray icon and a
/// notification without one are both cosmetic, and neither is worth a
/// dialog on a surface that may not exist in the first place.
pub(crate) fn install_icons(scheme: mark::Scheme) -> Option<&'static Icons> {
    ICONS
        .get_or_init(|| {
            let root = dirs::data_dir()?.join("trace-commons-shell").join("icons");
            write_theme(&root, scheme).ok()
        })
        .as_ref()
}

/// What [`install_icons`] wrote, for a caller that cannot read the scheme
/// itself. `None` until the main thread has installed them.
pub(crate) fn icons() -> Option<&'static Icons> {
    ICONS.get()?.as_ref()
}

/// The two mark variants plus the index that makes them a theme.
///
/// The layout is the freedesktop icon theme specification's, because that
/// is what a host will walk: `<root>/index.theme` naming the directories,
/// and one scalable directory per context. `scalable` and not a size ladder
/// -- the mark is geometry, and an SVG is the whole point of drawing it
/// rather than shipping it.
fn write_theme(root: &Path, scheme: mark::Scheme) -> std::io::Result<Icons> {
    let apps = root.join("scalable").join("apps");
    let status = root.join("scalable").join("status");
    std::fs::create_dir_all(&apps)?;
    std::fs::create_dir_all(&status)?;
    std::fs::write(root.join("index.theme"), index_theme())?;

    let app_icon = apps.join(format!("{}.svg", crate::ui::APP_ID));
    std::fs::write(&app_icon, mark::svg(scheme, MARK_SVG_SIZE))?;

    // The template variant carries the ink of the current scheme, rather
    // than `currentColor` for the host to resolve. A GTK host recolours a
    // symbolic icon by overriding `fill`, and the mark is drawn entirely
    // in strokes, so that override would pass it by; a host that does not
    // recolour at all would resolve `currentColor` to black and lose the
    // mark on a dark panel. Naming the scheme's own ink is right in both
    // cases, and matches what the window is drawing at the same moment.
    // The cost is that a scheme change after startup does not reach the
    // tray until the next run: following it would mean rewriting this file
    // and emitting `NewIcon` from the main thread into the D-Bus one, and
    // a bonus surface does not justify that machinery yet.
    let status_name = format!("{}-symbolic", crate::ui::APP_ID);
    std::fs::write(
        status.join(format!("{status_name}.svg")),
        mark::template_svg(scheme.ink(), MARK_SVG_SIZE),
    )?;

    Ok(Icons {
        theme_root: root.to_path_buf(),
        app_icon,
        status_name,
    })
}

/// The size written into the SVG's `width`/`height`. The geometry stays on
/// its 64-unit `viewBox` whatever this is, so it only sets the intrinsic
/// size a consumer starts from; the tray renders it at 14 or 16 and the
/// notification at whatever the daemon uses.
const MARK_SVG_SIZE: u32 = 64;

fn index_theme() -> String {
    format!(
        "[Icon Theme]\n\
         Name={name}\n\
         Comment=The Trace Commons mark, written at runtime\n\
         Directories=scalable/apps,scalable/status\n\
         \n\
         [scalable/apps]\n\
         Size=64\n\
         MinSize=8\n\
         MaxSize=512\n\
         Type=Scalable\n\
         Context=Applications\n\
         \n\
         [scalable/status]\n\
         Size=16\n\
         MinSize=8\n\
         MaxSize=512\n\
         Type=Scalable\n\
         Context=Status\n",
        name = crate::copy::APP_NAME,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!("trace-commons-tray-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn the_theme_holds_both_variants_of_the_mark() {
        let root = scratch();
        let icons = write_theme(&root, mark::Scheme::Light).expect("theme written");

        let index = std::fs::read_to_string(root.join("index.theme")).expect("index");
        assert!(index.contains("Directories=scalable/apps,scalable/status"));

        // The framed variant for the notification, the frameless one for the
        // tray -- the spec's rule for a status area, not a preference.
        let app = std::fs::read_to_string(&icons.app_icon).expect("app icon");
        assert!(app.contains(r##"<rect x="1" y="1""##));
        let status = std::fs::read_to_string(
            root.join("scalable")
                .join("status")
                .join(format!("{}.svg", icons.status_name)),
        )
        .expect("status icon");
        assert!(!status.contains("<rect"));
        assert_eq!(status.matches(r##"stroke-width="8""##).count(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_status_icon_is_named_without_a_path_or_an_extension() {
        let root = scratch();
        let icons = write_theme(&root, mark::Scheme::Dark).expect("theme written");
        // The tray protocol accepts a bare theme name and nothing else.
        assert!(!icons.status_name.contains('/'));
        assert!(!icons.status_name.ends_with(".svg"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_dark_scheme_reaches_the_ink() {
        let root = scratch();
        write_theme(&root, mark::Scheme::Dark).expect("theme written");
        let status = std::fs::read_to_string(
            root.join("scalable")
                .join("status")
                .join(format!("{}-symbolic.svg", crate::ui::APP_ID)),
        )
        .expect("status icon");
        assert!(status.contains(mark::Scheme::Dark.ink()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
