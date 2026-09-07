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
//! and registers it, with a `com.canonical.dbusmenu` menu beside it. The
//! icon does almost one kind of thing: a primary or secondary click raises
//! the window at its first screen, and a menu press raises it at the screen
//! that was pressed. `notify.rs`'s rule holds -- nothing reachable from
//! outside the window may approve or send anything.
//!
//! ## The one thing that is not navigation
//!
//! The menu can stop this computer answering model calls, and it cannot
//! start it. The two directions are not symmetrical and the menu's shape
//! says so: turning it OFF only ever reduces what this computer will answer,
//! so it is safe to press with nothing else on screen, while turning it ON
//! changes what anything else running here may send through, charged to the
//! contributor's own accounts. The sentence that says so is the reason model
//! calls became a screen of its own rather than a switch, and a menu press
//! that enabled it would route around that sentence. So the row says one of
//! two things and does one of two things: stop answering, or open the screen
//! where the switch and that sentence are side by side.
//!
//! This module still writes nothing itself. It asks, in the two words
//! [`TrayRequest`] has, and `ui::App` is what calls the daemon.
//!
//! The menu is generated from `ui::SCREENS` rather than listed here, so it
//! cannot offer a screen the window does not have, and every word in it is
//! the window's own. It is a shortcut in, never a second way to do
//! something: on plain GNOME there is no tray at all, so anything only
//! reachable from here would be unreachable for most contributors.
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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use zbus::interface;
use zbus::zvariant;

use crate::ui::mark;

/// Where the menu object lives on this connection. Named once: the item's
/// `Menu` property and the object server's registration have to agree, and
/// a host that is handed a path nothing is exported at simply shows no
/// menu.
const MENU_PATH: &str = "/MenuBar";

/// What a press asks the window for.
///
/// Two words and no more. [`Self::Open`] is what this surface has always
/// done; [`Self::StopAnsweringModelCalls`] is the single exception, and it
/// exists in one direction only -- see the module header. There is
/// deliberately no variant that starts answering: the type is where that
/// promise is kept, because a comment can be edited past without anything
/// noticing and a missing variant cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayRequest {
    /// Raise the window at one screen. Writes nothing.
    Open(&'static str),
    /// Stop answering model calls on this computer.
    StopAnsweringModelCalls,
}

/// Where the switch is, as the tray sees it.
///
/// Shared with the window, which writes it on every settings read. The tray
/// reads it to decide which way the action row points -- and only that. It
/// is the switch's own position, never a claim that a listener is running,
/// so nothing that paints may read it: on this surface only the `Clear` tone
/// may be drawn as working, and this menu draws no indicator at all.
#[derive(Clone, Default)]
pub struct TrayState {
    answering: Arc<AtomicBool>,
}

impl TrayState {
    /// The confirmed switch position, from the window.
    pub fn set_answering(&self, on: bool) {
        self.answering.store(on, Ordering::Relaxed);
    }

    /// Which way the one action points, for the menu here and for the
    /// window's toggle accelerator. Never a claim that a listener is
    /// running -- see the type's own note.
    pub fn answering(&self) -> bool {
        self.answering.load(Ordering::Relaxed)
    }
}

/// What "toggle answering" means, in one place.
///
/// **The whole of the asymmetry lives here**, and both surfaces that offer a
/// toggle ask this rather than restating it: the menu row below, and the
/// window's keyboard chord (`ui::shortcuts`). Turning it OFF only ever
/// reduces what this computer will answer, so it is safe to press with
/// nothing else on screen. Turning it ON changes what anything else running
/// here may send through, charged to the contributor's own accounts -- so
/// this never answers with a write for that direction. It answers with an
/// open, and the screen it opens is the one carrying the sentence that says
/// what the exposure is.
///
/// [`TrayRequest`] having no word for the other direction is what makes that
/// unforgeable rather than merely intended: this function cannot return one.
pub fn toggle_request(answering: bool) -> TrayRequest {
    if answering {
        TrayRequest::StopAnsweringModelCalls
    } else {
        TrayRequest::Open(crate::ui::PRIVATE_INFERENCE_SCREEN)
    }
}

/// The menu, as `(label, request)` in the order it is shown.
///
/// The screens come from `ui::SCREENS` rather than being listed here, so a
/// screen added to the window arrives in this menu without this module being
/// edited, and no item can ever name a screen the stack does not carry. The
/// labels are the window's own -- the model-calls ones are the shared copy
/// module's words, which is the only place that wording is allowed to be
/// decided.
///
/// The last row is the action, and which way it points is the whole of the
/// asymmetry: while it is off it is another `Open`.
fn menu_rows(answering: bool) -> Vec<(&'static str, TrayRequest)> {
    let mut rows: Vec<(&'static str, TrayRequest)> = crate::ui::SCREENS
        .iter()
        .map(|(name, label, _)| (*label, TrayRequest::Open(name)))
        .collect();
    // Which request the row carries is `toggle_request`'s decision and is
    // not made again here; this only picks the words for the request it was
    // handed. The trailing ellipsis on the open one is the convention for an
    // item that opens something rather than acting, and it is the copy
    // module's, not this file's.
    let request = toggle_request(answering);
    rows.push((
        match request {
            TrayRequest::StopAnsweringModelCalls => crate::copy::PRIVATE_INFERENCE_TRAY_TURN_OFF,
            TrayRequest::Open(_) => crate::copy::PRIVATE_INFERENCE_TRAY_OPEN_TO_TURN_ON,
        },
        request,
    ));
    rows
}

/// dbusmenu numbers items from 1: id 0 is the root the menu hangs off.
fn menu_id(index: usize) -> i32 {
    index as i32 + 1
}

/// What an id asks for, or `None` for an id this menu never handed out.
///
/// `None` rather than a default on purpose: a host that asks about an id we
/// do not know is confused, and opening whichever screen happened to be
/// first would be this application acting on a request nobody made.
///
/// Resolved against the switch position at PRESS time, not at layout time.
/// A host holding a stale menu therefore gets what the row would say now
/// rather than what it said when it was drawn -- and because the on
/// direction is an open, the worst a stale press can do is raise a window.
fn menu_request(id: i32, answering: bool) -> Option<TrayRequest> {
    let index = usize::try_from(id.checked_sub(1)?).ok()?;
    menu_rows(answering).get(index).map(|(_, request)| *request)
}

/// The screen a plain click opens: the first one the window lists.
fn default_screen() -> &'static str {
    crate::ui::SCREENS[0].0
}

/// What the tray icon does. `notify.rs` has the same shape for the same
/// reason: a surface reachable when the contributor is not looking at the
/// window must have the smallest possible vocabulary, and here the whole of
/// it is "open the window", optionally at a named screen.
///
/// **Navigation, plus the one write that only ever reduces what this
/// computer answers.** No item on the menu approves anything or sends
/// anything, and none of them can start answering model calls -- that stays
/// on the screen this menu opens, beside the sentence about what it exposes.
/// GNOME has no tray at all, so anything only reachable from here would be
/// unreachable for most contributors; see `ui/mod.rs`.
struct Item {
    tx: async_channel::Sender<TrayRequest>,
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

    /// The menu object this item hangs off, exported at [`MENU_PATH`].
    ///
    /// A host that reads this shows the screen list on a right-click; one
    /// that ignores the property calls [`Self::context_menu`] below instead
    /// and gets the window at its first screen, which is what it got before
    /// the menu existed.
    #[zbus(property)]
    fn menu(&self) -> zbus::fdo::Result<zvariant::OwnedObjectPath> {
        zvariant::ObjectPath::try_from(MENU_PATH)
            .map(Into::into)
            .map_err(|_| zbus::fdo::Error::Failed("tray-menu-path".into()))
    }

    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(TrayRequest::Open(default_screen()));
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(TrayRequest::Open(default_screen()));
    }

    /// The fallback for a host that does not read [`Self::menu`]: raise the
    /// window, which is the same thing every path here does and what the
    /// shared spec expects a tray-less desktop to reach anyway.
    fn context_menu(&self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(TrayRequest::Open(default_screen()));
    }
}

/// One property map for one menu item.
type MenuProps = std::collections::HashMap<String, zvariant::OwnedValue>;

/// dbusmenu's `(ia{sv}av)`: an id, its properties, and its children.
type MenuLayout = (i32, MenuProps, Vec<zvariant::OwnedValue>);

/// The screen list, as a menu.
///
/// Exported alongside the item because `StatusNotifierItem` names a menu by
/// object path and carries none itself. Every entry but the last raises the
/// window at one screen; the last stops this computer answering model calls,
/// or -- while it is already off -- opens the screen where the switch and
/// the sentence about what it exposes are side by side. There is no toggle
/// here and there must not be one: a toggle is symmetrical and these two
/// directions are not.
struct Menu {
    tx: async_channel::Sender<TrayRequest>,
    /// The switch position, shared with the window.
    answering: TrayState,
    /// What the last layout this menu served said. Compared against
    /// [`Self::answering`] in `AboutToShow`, which is the protocol's own
    /// way of telling a host its cached menu is stale.
    served: AtomicBool,
    /// Moves whenever the layout stops matching what was served, so a host
    /// that caches by revision refetches.
    revision: AtomicU32,
}

impl Menu {
    fn new(tx: async_channel::Sender<TrayRequest>, answering: TrayState) -> Self {
        let served = answering.answering();
        Self {
            tx,
            answering,
            served: AtomicBool::new(served),
            revision: AtomicU32::new(1),
        }
    }
}

fn menu_props(label: &str) -> MenuProps {
    let mut props = MenuProps::new();
    // `zvariant`'s conversions from a string and a bool are infallible in
    // practice, but the fallible form is the one the API offers; an entry
    // that somehow failed to build is simply left off the menu rather than
    // panicking a worker thread inside a bonus surface.
    if let Ok(label) = zvariant::Value::from(label.to_string()).try_into() {
        props.insert("label".to_string(), label);
    }
    for flag in ["enabled", "visible"] {
        if let Ok(value) = zvariant::Value::from(true).try_into() {
            props.insert(flag.to_string(), value);
        }
    }
    props
}

fn menu_layout(answering: bool) -> MenuLayout {
    let children = menu_rows(answering)
        .into_iter()
        .enumerate()
        .filter_map(|(index, (label, _))| {
            // `(ia{sv}av)`, the child shape dbusmenu's layout is made of.
            // A leaf, so its own children list is empty.
            let item = zvariant::Structure::from((
                menu_id(index),
                menu_props(label),
                Vec::<zvariant::OwnedValue>::new(),
            ));
            zvariant::OwnedValue::try_from(zvariant::Value::from(item)).ok()
        })
        .collect();
    let mut root = MenuProps::new();
    if let Ok(display) = zvariant::Value::from("submenu".to_string()).try_into() {
        root.insert("children-display".to_string(), display);
    }
    (0, root, children)
}

#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    /// The items carry no icons, so there is no extra theme to search. The
    /// item's own icon is [`Item::icon_theme_path`]'s business.
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    /// The layout is fixed for the life of the process -- it is the window's
    /// screen list -- so the revision never moves and a host may cache it.
    /// The layout for the switch position as it stands now, at the revision
    /// that position was last given.
    fn get_layout(
        &self,
        _parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, MenuLayout) {
        let answering = self.answering.answering();
        self.served.store(answering, Ordering::Relaxed);
        (
            self.revision.load(Ordering::Relaxed),
            menu_layout(answering),
        )
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, MenuProps)> {
        menu_rows(self.answering.answering())
            .into_iter()
            .enumerate()
            .filter(|(index, _)| ids.is_empty() || ids.contains(&menu_id(*index)))
            .map(|(index, (label, _))| (menu_id(index), menu_props(label)))
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<zvariant::OwnedValue> {
        let answering = self.answering.answering();
        let index = usize::try_from(id.checked_sub(1).unwrap_or(-1))
            .ok()
            .filter(|index| *index < menu_rows(answering).len())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("tray-menu-id".into()))?;
        let label = menu_rows(answering)[index].0;
        menu_props(label)
            .remove(&name)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("tray-menu-property".into()))
    }

    /// A click on one entry, and nothing else. Every other event id --
    /// hover, opened, closed -- is ignored rather than treated as a press.
    ///
    /// The request is resolved against the switch position NOW rather than
    /// against the layout the host is holding. A menu drawn while it was on
    /// and pressed after it went off asks for an open, which is the safe
    /// direction; the reverse cannot happen, because the off layout has no
    /// row that writes.
    fn event(&self, id: i32, event_id: String, _data: zvariant::OwnedValue, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        if let Some(request) = menu_request(id, self.answering.answering()) {
            let _ = self.tx.send_blocking(request);
        }
    }

    /// Whether the host's cached menu is stale, which is the one thing this
    /// call is for. True exactly when the switch has moved since the layout
    /// it is holding was served: the labels differ between the two
    /// positions, and a host that redrew the old one would name an action
    /// the press will not take.
    fn about_to_show(&self, _id: i32) -> bool {
        let answering = self.answering.answering();
        if self.served.swap(answering, Ordering::Relaxed) == answering {
            return false;
        }
        self.revision.fetch_add(1, Ordering::Relaxed);
        true
    }
}

/// Try to become a tray icon, in the background. Never blocks the caller
/// and never reports failure anywhere a contributor would see it: absence
/// of a watcher is the normal case on the majority desktop, not an error.
///
/// One [`TrayRequest`] is sent per click or menu press, for as long as the
/// process lives. The receiving end (see `ui::App`) is what raises the
/// window and what calls the daemon; this module only produces the signal,
/// and every screen it can name comes from `ui::SCREENS`.
///
/// `answering` is the switch position, written by the window on every
/// settings read. It decides which way the action row points and nothing
/// else.
pub fn spawn(answering: TrayState) -> async_channel::Receiver<TrayRequest> {
    // On the caller's thread, which is the main one: the ink of the mark
    // follows the desktop's light/dark preference, and `adw::StyleManager`
    // may only be read from the main thread. Everything after this point
    // runs on a thread that must never touch GTK, so the icons are written
    // -- and their paths frozen -- here.
    let icons = install_icons(mark::current_scheme());
    let icon_name = icons
        .map(|icons| icons.status_name.clone())
        .unwrap_or_else(|| crate::ui::APP_ID.to_string());
    let icon_theme_path = icons
        .map(|icons| icons.theme_root.to_string_lossy().into_owned())
        .unwrap_or_default();

    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        if let Err(_error) = register(tx, answering, icon_name, icon_theme_path) {
            // Fixed label. No watcher (plain GNOME, most Linux desktops
            // today) and a watcher that rejects the registration both land
            // here, and neither is a bug in this application.
            eprintln!("trace-commons-shell: tray unavailable");
        }
    });
    rx
}

fn register(
    tx: async_channel::Sender<TrayRequest>,
    answering: TrayState,
    icon_name: String,
    icon_theme_path: String,
) -> anyhow::Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    connection.object_server().at(
        "/StatusNotifierItem",
        Item {
            tx: tx.clone(),
            icon_name,
            icon_theme_path,
        },
    )?;
    // Exported before the watcher is told about the item, so a host that
    // reads `Menu` the moment it is registered finds the object already
    // there rather than a path with nothing at it.
    connection
        .object_server()
        .at(MENU_PATH, Menu::new(tx, answering))?;

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
         Comment=The {name} mark, written at runtime\n\
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

    /// The menu is built from the window's own screen list, so an item can
    /// never offer a screen the window does not have -- and a screen added
    /// to the window arrives here without this module being edited. One row
    /// beyond the screens: the model-calls action, which is the only thing
    /// in this menu that is not simply a screen.
    #[test]
    fn the_menu_offers_one_item_per_screen_and_one_action() {
        for answering in [false, true] {
            let rows = menu_rows(answering);
            assert_eq!(rows.len(), crate::ui::SCREENS.len() + 1);
            for (id, (label, request)) in rows.iter().take(crate::ui::SCREENS.len()).enumerate() {
                let (screen, title, _) = crate::ui::SCREENS[id];
                assert_eq!(*request, TrayRequest::Open(screen));
                assert_eq!(*label, title);
            }
        }
    }

    /// The model-calls screen is reachable from the tray, and its label is
    /// the shared copy module's word rather than one written here.
    #[test]
    fn the_model_calls_screen_is_one_of_the_items() {
        let rows = menu_rows(false);
        let (label, _) = rows
            .iter()
            .find(|(_, request)| *request == TrayRequest::Open(crate::ui::PRIVATE_INFERENCE_SCREEN))
            .expect("the tray offers the model-calls screen");
        assert_eq!(*label, crate::copy::PRIVATE_INFERENCE_DESTINATION);
    }

    /// Every menu id resolves to a request this menu handed out, and nothing
    /// else. An id a host invented resolves to nothing rather than to
    /// whichever row happened to be first.
    #[test]
    fn only_a_known_id_asks_for_anything() {
        for answering in [false, true] {
            for (index, (_, request)) in menu_rows(answering).iter().enumerate() {
                assert_eq!(menu_request(menu_id(index), answering), Some(*request));
            }
            let past_the_end = menu_rows(answering).len() as i32 + 1;
            for stray in [0, -1, i32::MAX, i32::MIN, past_the_end] {
                assert_eq!(
                    menu_request(stray, answering),
                    None,
                    "id {stray} must not ask for anything"
                );
            }
        }
    }

    /// The tray may turn model calls OFF and may not turn them ON.
    ///
    /// Turning it off only ever reduces what this computer will answer, so
    /// it is safe from a menu with nothing else on screen. Turning it on
    /// changes what anything else running here may send through, charged to
    /// the contributor's own accounts, and the sentence saying so is on the
    /// screen this row opens. So while it is off the tray's entire
    /// vocabulary is still navigation -- asserted over every row and every
    /// id, not just the one that changed.
    #[test]
    fn the_tray_can_stop_answering_and_can_never_start() {
        for (_, request) in menu_rows(false) {
            assert!(
                matches!(request, TrayRequest::Open(_)),
                "the tray asked for a write while model calls were off"
            );
        }
        for id in -3..12 {
            assert!(
                !matches!(
                    menu_request(id, false),
                    Some(TrayRequest::StopAnsweringModelCalls)
                ),
                "id {id} asked for a write while model calls were off"
            );
        }

        let (label, request) = *menu_rows(true).last().expect("the action row");
        assert_eq!(request, TrayRequest::StopAnsweringModelCalls);
        assert_eq!(label, crate::copy::PRIVATE_INFERENCE_TRAY_TURN_OFF);
        assert_eq!(
            menu_rows(false).last().expect("the action row").0,
            crate::copy::PRIVATE_INFERENCE_TRAY_OPEN_TO_TURN_ON
        );
        assert_eq!(
            menu_rows(false).last().expect("the action row").1,
            TrayRequest::Open(crate::ui::PRIVATE_INFERENCE_SCREEN)
        );
    }

    /// The action row keeps one id whichever way it is pointing, so a host
    /// that cached the layout cannot press one row and reach another; and
    /// the label is refetched when the switch moved, which is what
    /// `AboutToShow` is for.
    #[test]
    fn a_stale_menu_is_refetched_rather_than_acted_on() {
        let action = menu_id(crate::ui::SCREENS.len());
        assert_eq!(
            menu_request(action, true),
            Some(TrayRequest::StopAnsweringModelCalls)
        );
        assert_eq!(
            menu_request(action, false),
            Some(TrayRequest::Open(crate::ui::PRIVATE_INFERENCE_SCREEN))
        );

        let (tx, _rx) = async_channel::unbounded();
        let state = TrayState::default();
        let menu = Menu::new(tx, state.clone());
        let (first, _) = menu.get_layout(0, -1, Vec::new());
        assert!(!menu.about_to_show(0), "nothing moved, nothing to refetch");
        state.set_answering(true);
        assert!(
            menu.about_to_show(0),
            "the switch moved and the menu did not"
        );
        let (second, _) = menu.get_layout(0, -1, Vec::new());
        assert!(second > first, "the layout revision did not move");
        assert!(!menu.about_to_show(0), "refetched twice for one change");
    }

    /// The tray's vocabulary is navigation plus exactly one write, and that
    /// write only ever REDUCES what this computer answers.
    ///
    /// This module still performs no write itself: it asks, in the two words
    /// [`TrayRequest`] has, and the window is what calls the daemon. So the
    /// scan below keeps forbidding the verbs it always forbade --
    /// `set_settings`, `approve`, `submit`, `withdraw` -- and the one new
    /// capability is fenced by type instead, in
    /// `the_tray_can_stop_answering_and_can_never_start`: no row and no id,
    /// in either state, can ask for anything but an open while model calls
    /// are off. Widening [`TrayRequest`] fails the exhaustive match here as
    /// well.
    #[test]
    fn the_tray_only_opens_the_window_or_stops_answering() {
        // Exhaustive on purpose: a third variant does not compile until
        // somebody comes back and argues for it here.
        for request in [
            TrayRequest::Open(default_screen()),
            TrayRequest::StopAnsweringModelCalls,
        ] {
            match request {
                TrayRequest::Open(screen) => {
                    assert!(
                        crate::ui::SCREENS
                            .iter()
                            .any(|(name, _, _)| *name == screen)
                    )
                }
                TrayRequest::StopAnsweringModelCalls => {}
            }
        }
        // Comments are where the rule is written down, and the tests are
        // where it is spelled out, so only the module's own code is
        // scanned for breaking it.
        let code: String = include_str!("tray.rs")
            .split("\n#[cfg(test)]")
            .next()
            .expect("the module has a body before its tests")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in ["set_settings", "approve", "submit", "withdraw"] {
            assert!(
                !code.contains(forbidden),
                "the tray must not be able to {forbidden}"
            );
        }
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
