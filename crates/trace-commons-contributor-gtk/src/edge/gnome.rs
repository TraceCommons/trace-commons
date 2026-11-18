//! The GNOME Shell extension's side of the bus, as this app sees it.
//!
//! GNOME has no layer-shell, and a GTK 4 window cannot place itself. The
//! extension in `gnome-shell-extension/` does it for this app's two
//! windows: it owns [`BUS_NAME`] on the session bus and exports one narrow
//! interface, [`INTERFACE`], at [`OBJECT_PATH`]:
//!
//! * `Place(s role, i x, i y, b open)`: the calling connection's window
//!   carrying `role`'s marker is put where `role` says -- `"rail"` against
//!   the primary monitor's right edge, centred, and kept there as it
//!   resizes; `"panel"` near (`x`, `y`) on the monitor holding it, or near
//!   the pointer when they are negative -- kept above other windows and
//!   shown on every workspace. `open` says whether the rail is open (the
//!   panel's is ignored): an open rail keeps keyboard focus once given it,
//!   a closed one never does (owner, 2026-11-18, "Keyboard only while
//!   open").
//! * `Release(s role)`: stop placing it.
//! * `Version` (u): the interface's version, [`VERSION`].
//!
//! Windows are told apart by a hidden marker, never by title (owner,
//! 2026-11-18): the rail, the panel and the main window are all titled
//! with the app's name, which is what Alt-Tab and the overview show. The
//! rail and the panel are application windows of this app ([`surface_window`])
//! and each carries a disabled action named for its role
//! ([`Role::marker`]). GTK exports a window's actions on the app's own bus
//! connection at the window's object path and tells Mutter that path and
//! connection; the extension reads the actions back from there and places
//! a window only if its role's marker is among them. The main window
//! carries none, so it is never touched, and the extension cannot be
//! pointed at another connection's windows. Nothing else crosses: no
//! trace, no setting, no word. Without the extension the app works as
//! before (`capability`): no rail, and a plain panel window.
//!
//! GNOME Shell disables every extension on the lock screen and enables it
//! again on unlock, and the extension undoes its placements when disabled.
//! So the client remembers the last placement asked for each window and
//! asks again whenever the extension's name gets a new owner.

use std::cell::RefCell;
use std::rc::Rc;

/// The extension's well-known name. Under the app's own id, so a Flatpak
/// sandbox may talk to it without a further hole.
pub const BUS_NAME: &str = "ai.tracecommons.Contributor.EdgeSurfaces";
pub const OBJECT_PATH: &str = "/ai/tracecommons/Contributor/EdgeSurfaces";
pub const INTERFACE: &str = "ai.tracecommons.Contributor.EdgeSurfaces1";
pub const VERSION: u32 = 3;

/// The rail's and the panel's markers: the names of the disabled actions
/// each carries (`placement.js`, `MARKER_RAIL` and `MARKER_PANEL`). Names,
/// not words: nothing shows them.
pub const RAIL_MARKER: &str = "edge-rail";
pub const PANEL_MARKER: &str = "menu-panel";

/// Which of the two windows a placement is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Rail,
    Panel,
}

impl Role {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Rail => "rail",
            Self::Panel => "panel",
        }
    }

    /// The marker the extension looks for on this role's window.
    pub fn marker(self) -> &'static str {
        match self {
            Self::Rail => RAIL_MARKER,
            Self::Panel => PANEL_MARKER,
        }
    }
}

/// A window the extension places for `role`: an application window of
/// `application`, so GTK exports its actions and tells Mutter where, titled
/// with the app's name like every window of the app, and carrying `role`'s
/// marker as a disabled action that does nothing.
pub fn surface_window(application: &gtk::Application, role: Role) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title(crate::copy::APP_NAME)
        .show_menubar(false)
        .build();
    let marker = gtk::gio::SimpleAction::new(role.marker(), None);
    marker.set_enabled(false);
    gtk::prelude::ActionMapExt::add_action(&window, &marker);
    window
}

/// One request to the extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Place {
        role: Role,
        /// Where the panel was asked for; negative for the pointer's
        /// position. Ignored for the rail.
        at: (i32, i32),
        /// Whether the rail is open. Ignored for the panel.
        open: bool,
    },
    Release {
        role: Role,
    },
}

/// The way to ask the extension for placements, once it is known to be
/// there. Calls go over GIO's session-bus connection on the main loop; a
/// failed call is dropped, and the window stays where the window manager
/// put it, which is the no-extension behaviour.
#[derive(Clone)]
pub struct Client {
    connection: gtk::gio::DBusConnection,
    /// What to ask again when the extension comes back.
    held: Rc<RefCell<Held>>,
}

impl Client {
    pub fn send(&self, request: Request) {
        self.held.borrow_mut().record(&request);
        call(&self.connection, &request);
    }

    /// Asks again for every placement still held, whenever the extension's
    /// name gets an owner: GNOME Shell disables every extension on the lock
    /// screen, the extension then undoes its placements, and enables it
    /// again on unlock. The watch lasts as long as the process.
    fn replay_on_return(&self) {
        let (connection, held) = (self.connection.clone(), Rc::downgrade(&self.held));
        // Never unwatched: one client per process, for its whole life. The
        // id has no destructor, so dropping it leaves the watch running.
        let _ = gtk::gio::bus_watch_name_on_connection(
            &self.connection,
            BUS_NAME,
            gtk::gio::BusNameWatcherFlags::NONE,
            move |_, _, _| {
                if let Some(held) = held.upgrade() {
                    for request in held.borrow().replay() {
                        call(&connection, request);
                    }
                }
            },
            |_, _| {},
        );
    }
}

/// One call, answer ignored. A failed call is dropped, and the window stays
/// where the window manager put it, which is the no-extension behaviour.
fn call(connection: &gtk::gio::DBusConnection, request: &Request) {
    use gtk::glib::variant::ToVariant;
    let (method, args) = match request {
        Request::Place { role, at, open } => {
            ("Place", (role.wire(), at.0, at.1, *open).to_variant())
        }
        Request::Release { role } => ("Release", (role.wire(),).to_variant()),
    };
    connection.call(
        Some(BUS_NAME),
        OBJECT_PATH,
        INTERFACE,
        method,
        Some(&args),
        None,
        gtk::gio::DBusCallFlags::NONE,
        -1,
        gtk::gio::Cancellable::NONE,
        |_| {},
    );
}

/// The placements to ask for again, by window: the last `Place` for each,
/// gone once released.
#[derive(Default, Debug, PartialEq, Eq)]
struct Held {
    places: std::collections::BTreeMap<&'static str, Request>,
}

impl Held {
    fn record(&mut self, request: &Request) {
        match request {
            Request::Place { role, .. } => {
                self.places.insert(role.wire(), request.clone());
            }
            Request::Release { role } => {
                self.places.remove(role.wire());
            }
        }
    }

    fn replay(&self) -> impl Iterator<Item = &Request> {
        self.places.values()
    }
}

/// The extension, if it owns its name on the session bus. `None` on a
/// session with no bus, or where it is not installed and enabled.
pub async fn find() -> Option<Client> {
    use gtk::glib::variant::ToVariant;
    let connection = gtk::gio::bus_get_future(gtk::gio::BusType::Session)
        .await
        .ok()?;
    let reply = connection
        .call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            Some(&(BUS_NAME,).to_variant()),
            Some(gtk::glib::VariantTy::new("(b)").ok()?),
            gtk::gio::DBusCallFlags::NONE,
            2000,
        )
        .await
        .ok()?;
    let (owned,) = reply.get::<(bool,)>()?;
    if !owned {
        return None;
    }
    let client = Client {
        connection,
        held: Rc::default(),
    };
    client.replay_on_return();
    Some(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension_file(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("gnome-shell-extension/edge-surfaces@tracecommons.ai")
                .join(name),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// The extension's source names the same bus name, path, interface,
    /// version, roles and margin. A rename on one side alone would leave
    /// the app calling an interface nothing exports.
    #[test]
    fn the_extension_exports_what_the_app_calls() {
        let extension = extension_file("extension.js");
        for needle in [
            format!("'{BUS_NAME}'"),
            format!("'{OBJECT_PATH}'"),
            format!("<interface name=\"{INTERFACE}\">"),
            format!("const VERSION = {VERSION};"),
            "<method name=\"Place\">".to_string(),
            "<arg type=\"s\" direction=\"in\" name=\"role\"/>".to_string(),
            "<arg type=\"b\" direction=\"in\" name=\"open\"/>".to_string(),
            "<method name=\"Release\">".to_string(),
        ] {
            assert!(extension.contains(&needle), "extension.js lacks {needle}");
        }
        let placement = extension_file("placement.js");
        for needle in [
            format!("ROLE_RAIL = '{}';", Role::Rail.wire()),
            format!("ROLE_PANEL = '{}';", Role::Panel.wire()),
            format!("MARGIN = {};", crate::edge::geometry::PANEL_MARGIN),
            format!("MARKER_RAIL = '{RAIL_MARKER}';"),
            format!("MARKER_PANEL = '{PANEL_MARKER}';"),
        ] {
            assert!(placement.contains(&needle), "placement.js lacks {needle}");
        }
    }

    /// What GNOME's Extensions app shows: the app's own name and the rail's
    /// own caption from the shared copy, never words of the extension's.
    #[test]
    fn the_extension_is_named_and_described_in_the_apps_words() {
        let metadata: serde_json::Value =
            serde_json::from_str(&extension_file("metadata.json")).expect("metadata.json parses");
        assert_eq!(metadata["name"], trace_commons_contributor::brand::APP_NAME);
        assert_eq!(
            metadata["description"],
            trace_commons_contributor::preview_copy::monitor_screens_copy()
                .edge_rail
                .setting_caption
        );
        // GNOME Shell 46 is Ubuntu 24.04's; 51 the newest written for.
        let versions = metadata["shell-version"].as_array().expect("shell-version");
        assert!(versions.iter().any(|v| v == "46"));
        assert!(versions.iter().any(|v| v == "51"));
    }

    #[test]
    fn a_returning_extension_is_asked_again_for_what_is_still_placed() {
        let place = |role, open| Request::Place {
            role,
            at: (-1, -1),
            open,
        };
        let mut held = Held::default();
        held.record(&place(Role::Rail, false));
        held.record(&place(Role::Panel, true));
        held.record(&Request::Place {
            role: Role::Panel,
            at: (5, 6),
            open: true,
        });
        held.record(&Request::Release { role: Role::Rail });
        let again: Vec<&Request> = held.replay().collect();
        assert_eq!(
            again,
            vec![&Request::Place {
                role: Role::Panel,
                at: (5, 6),
                open: true,
            }],
            "the last placement per window, and nothing released"
        );
        // The rail's open state is the latest asked, so an unlock while it
        // is open gives it back its keyboard.
        let mut held = Held::default();
        held.record(&place(Role::Rail, false));
        held.record(&place(Role::Rail, true));
        assert_eq!(
            held.replay().collect::<Vec<_>>(),
            vec![&place(Role::Rail, true)]
        );
    }

    /// The windows the extension places carry their role's marker, and
    /// only it, as an action that does nothing; they are titled with the
    /// app's name, as the main window is, so the title tells nothing apart
    /// (owner, 2026-11-18). An application window without the marker -- the
    /// main window's shape -- carries neither. The extension's half, that
    /// it places only a marked window, is `tests/placement.test.js` and the
    /// live test below.
    #[test]
    #[ignore = "requires a GTK display; exercised by weston"]
    fn mounted_the_placed_windows_carry_their_marker_and_the_brand_name() {
        use gtk::prelude::*;
        gtk::init().expect("GTK display");
        let application = gtk::Application::new(
            Some("ai.tracecommons.Contributor.MarkerTest"),
            gtk::gio::ApplicationFlags::NON_UNIQUE,
        );
        application
            .register(None::<&gtk::gio::Cancellable>)
            .expect("registered");
        let main = gtk::ApplicationWindow::builder()
            .application(&application)
            .title(crate::copy::APP_NAME)
            .build();
        for role in [Role::Rail, Role::Panel] {
            let window = surface_window(&application, role);
            assert_eq!(window.title().as_deref(), Some(crate::copy::APP_NAME));
            assert!(!window.shows_menubar());
            assert_eq!(window.application().as_ref(), Some(&application));
            let marker = window.lookup_action(role.marker()).expect("the marker");
            assert!(!marker.is_enabled(), "{role:?}'s marker can be activated");
            let other = match role {
                Role::Rail => Role::Panel,
                Role::Panel => Role::Rail,
            };
            assert!(window.lookup_action(other.marker()).is_none());
            assert!(
                !window
                    .list_actions()
                    .iter()
                    .any(|name| name == other.marker())
            );
            window.destroy();
        }
        for role in [Role::Rail, Role::Panel] {
            assert!(main.lookup_action(role.marker()).is_none());
        }
        assert_ne!(RAIL_MARKER, PANEL_MARKER);
        main.destroy();
    }

    /// The zip for extensions.gnome.org holds exactly what the shell loads:
    /// the three files, and every module extension.js imports.
    #[test]
    fn the_zip_carries_every_file_the_shell_loads() {
        let script = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("scripts/pack-gnome-shell-extension.sh"),
        )
        .expect("pack script");
        let mut packed: Vec<String> = script
            .split("files=(")
            .nth(1)
            .and_then(|rest| rest.split(')').next())
            .expect("a files=( ... ) list")
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let extension = extension_file("extension.js");
        let mut loaded = vec!["metadata.json".to_string(), "extension.js".to_string()];
        for part in extension.split("from './").skip(1) {
            loaded.push(part.split('\'').next().expect("import").to_string());
        }
        assert!(loaded.contains(&"placement.js".to_string()));
        packed.sort();
        loaded.sort();
        assert_eq!(packed, loaded);
        assert!(
            script.contains("gnome-shell-extension/edge-surfaces@tracecommons.ai"),
            "the script packs this extension's directory"
        );
    }

    #[test]
    fn the_bus_name_sits_under_the_apps_id() {
        assert!(BUS_NAME.starts_with(&format!("{}.", crate::ui::APP_ID)));
        assert_eq!(Role::Rail.wire(), "rail");
        assert_eq!(Role::Panel.wire(), "panel");
    }
}

/// The extension and this client against a real GNOME Shell.
///
/// Run by `scripts/gnome-shell-extension-verify.sh`: it starts a headless
/// shell on a private bus with the extension enabled, plus a throwaway
/// extension that turns on the shell's unsafe mode so `org.gnome.Shell.Eval`
/// can read Mutter's answer back. No CI runner has GNOME Shell, and weston
/// has none of its API, so the weston job does not run this.
///
/// The windows are shaped as the app's are: application windows of one
/// registered application, all titled with the app's name, the rail and
/// the panel made by [`surface_window`] and a main window without a marker.
/// They are told apart here by their object paths, as the extension tells
/// them apart by their markers. Every step waits for what it expects,
/// polling up to a deadline, rather than for a fixed time: GNOME Shell 46
/// settles slower than 51.
///
/// The expected frames are restated here, independently of `placement.js`,
/// so a wrong port there shows up as a disagreement: the rail by the Mac's
/// `EdgeRailGeometry.frame` (right work-area edge, centred, no taller than
/// the work area), the panel by `geometry::panel_anchor`'s rule.
#[cfg(test)]
#[cfg(target_os = "linux")]
mod live_tests {

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use gtk::glib;
    use gtk::prelude::*;

    use super::*;
    use crate::edge::geometry;

    const UUID: &str = "edge-surfaces@tracecommons.ai";
    /// How long any one expectation may take to come true.
    const DEADLINE: Duration = Duration::from_secs(15);
    const POLL: Duration = Duration::from_millis(100);

    /// One of this process's windows as Mutter sees it.
    #[derive(Clone, Debug, PartialEq, serde::Deserialize)]
    struct Seen {
        path: String,
        title: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        above: bool,
        sticky: bool,
        shown: bool,
    }

    #[derive(Debug, serde::Deserialize)]
    struct Shell {
        work_area: [i32; 4],
        windows: Vec<Seen>,
    }

    /// The focused window's path (empty for none), whether the main window
    /// is minimised, and the active workspace's index.
    #[derive(Debug, serde::Deserialize)]
    struct Focus {
        focused: String,
        minimised: bool,
        workspace: i32,
    }

    async fn eval(connection: &gtk::gio::DBusConnection, js: &str) -> Result<String, String> {
        let answer = connection
            .call_future(
                Some("org.gnome.Shell"),
                "/org/gnome/Shell",
                "org.gnome.Shell",
                "Eval",
                Some(&(js,).to_variant()),
                Some(glib::VariantTy::new("(bs)").expect("type")),
                gtk::gio::DBusCallFlags::NONE,
                5_000,
            )
            .await
            .map_err(|e| format!("Eval: {e}"))?;
        let (ok, text) = answer.get::<(bool, String)>().ok_or("Eval answer")?;
        if ok {
            Ok(text)
        } else {
            Err(format!("Eval failed: {text}"))
        }
    }

    /// Runs `body` with `w(path)` naming this process's window at that
    /// object path and `now` the display's time, and answers with what it
    /// returns, as a string.
    async fn with_windows(
        connection: &gtk::gio::DBusConnection,
        body: &str,
    ) -> Result<String, String> {
        let text = eval(
            connection,
            &format!(
                "(() => {{ const w = p => global.get_window_actors().map(a => a.meta_window) \
                 .find(x => x.get_pid() === {pid} && x.get_gtk_window_object_path() === p); \
                 const now = global.display.get_current_time_roundtrip(); \
                 return String((() => {{ {body} }})()); }})()",
                pid = std::process::id(),
            ),
        )
        .await?;
        serde_json::from_str(&text).map_err(|e| format!("{e}: {text}"))
    }

    async fn look(connection: &gtk::gio::DBusConnection) -> Result<Shell, String> {
        let text = with_windows(
            connection,
            "const m = global.display.get_primary_monitor(); \
             const wa = global.workspace_manager.get_active_workspace().get_work_area_for_monitor(m); \
             return JSON.stringify({ work_area: [wa.x, wa.y, wa.width, wa.height], \
               windows: global.get_window_actors().map(a => a.meta_window) \
                 .filter(x => x.get_pid() === PID) \
                 .map(x => { const r = x.get_frame_rect(); return { \
                   path: x.get_gtk_window_object_path() || '', title: x.get_title() || '', \
                   x: r.x, y: r.y, width: r.width, height: r.height, \
                   above: x.is_above(), sticky: x.is_on_all_workspaces(), \
                   shown: !x.is_hidden() }; }) });"
                .replace("PID", &std::process::id().to_string())
                .as_str(),
        )
        .await?;
        serde_json::from_str(&text).map_err(|e| format!("{e}: {text}"))
    }

    async fn focus_state(
        connection: &gtk::gio::DBusConnection,
        main: &str,
    ) -> Result<Focus, String> {
        let text = with_windows(
            connection,
            &format!(
                "const f = global.display.focus_window; \
                 return JSON.stringify({{ focused: f ? (f.get_gtk_window_object_path() || '') : '', \
                 minimised: w('{main}').minimized, \
                 workspace: global.workspace_manager.get_active_workspace_index() }});"
            ),
        )
        .await?;
        serde_json::from_str(&text).map_err(|e| format!("{e}: {text}"))
    }

    /// Looks until `expect` holds of the shell, and answers with that look;
    /// past the deadline, fails with `what` and the last look.
    async fn until(
        connection: &gtk::gio::DBusConnection,
        what: &str,
        expect: impl Fn(&Shell) -> bool,
    ) -> Result<Shell, String> {
        let start = Instant::now();
        loop {
            let shell = look(connection).await?;
            if expect(&shell) {
                return Ok(shell);
            }
            if start.elapsed() > DEADLINE {
                return Err(format!("{what}: {shell:?}"));
            }
            glib::timeout_future(POLL).await;
        }
    }

    /// The same for keyboard focus.
    async fn until_focus(
        connection: &gtk::gio::DBusConnection,
        main: &str,
        what: &str,
        expect: impl Fn(&Focus) -> bool,
    ) -> Result<Focus, String> {
        let start = Instant::now();
        loop {
            let focus = focus_state(connection, main).await?;
            if expect(&focus) {
                return Ok(focus);
            }
            if start.elapsed() > DEADLINE {
                return Err(format!("{what}: {focus:?}"));
            }
            glib::timeout_future(POLL).await;
        }
    }

    fn window<'a>(shell: &'a Shell, path: &str) -> Option<&'a Seen> {
        shell.windows.iter().find(|w| w.path == path)
    }

    /// The Mac's rail frame: against the right edge, centred, never taller.
    fn rail_ok(shell: &Shell, path: &str) -> bool {
        let [x, y, w, h] = shell.work_area;
        window(shell, path).is_some_and(|rail| {
            rail.shown
                && rail.height <= h
                && rail.x == x + w - rail.width
                && rail.y == y + (h - rail.height).div_euclid(2)
        })
    }

    /// `geometry::panel_anchor`'s rule for a press at `at`: centred on it
    /// across, inside the margin, and against the top or bottom margin by the
    /// half it is in.
    fn panel_ok(shell: &Shell, path: &str, at: (i32, i32)) -> bool {
        let [x, y, w, h] = shell.work_area;
        let margin = geometry::PANEL_MARGIN;
        window(shell, path).is_some_and(|panel| {
            let left = (at.0 - panel.width / 2)
                .max(x + margin)
                .min((x + margin).max(x + w - panel.width - margin));
            let top = if at.1 * 2 < 2 * y + h {
                y + margin
            } else {
                y + h - panel.height - margin
            };
            panel.shown && (panel.x, panel.y) == (left, top)
        })
    }

    /// The main window exactly as it was: never moved, kept above or stuck.
    fn main_untouched(shell: &Shell, main: &Seen) -> Result<(), String> {
        match window(shell, &main.path) {
            Some(now) if now == main => Ok(()),
            now => Err(format!(
                "the main window was touched: {now:?}, was {main:?}"
            )),
        }
    }

    const PANEL_AT: (i32, i32) = (1700, 10);

    struct Windows {
        main: String,
        rail: String,
        panel: String,
    }

    fn path_of(application: &gtk::Application, window: &gtk::ApplicationWindow) -> String {
        format!(
            "{}/window/{}",
            application
                .dbus_object_path()
                .expect("an exported application"),
            window.id()
        )
    }

    async fn run(
        connection: &gtk::gio::DBusConnection,
        client: &Client,
        application: &gtk::Application,
        main_window: &gtk::ApplicationWindow,
    ) -> Result<(), String> {
        let place_rail = |open| Request::Place {
            role: Role::Rail,
            at: (-1, -1),
            open,
        };
        let place_panel = || Request::Place {
            role: Role::Panel,
            at: PANEL_AT,
            open: true,
        };
        let main = path_of(application, main_window);

        // The main window alone, settled where Mutter put it.
        let shell = until(connection, "the main window shown", |shell| {
            window(shell, &main).is_some_and(|w| w.shown)
        })
        .await?;
        let mut before = window(&shell, &main).cloned().ok_or("main window")?;
        loop {
            glib::timeout_future(Duration::from_millis(300)).await;
            let now = look(connection).await?;
            let now = window(&now, &main).cloned().ok_or("main window")?;
            if now == before {
                break;
            }
            before = now;
        }
        let main_seen = before;

        // Both placements asked for while the main window -- the same
        // process, the same connection, the same title -- is the only window
        // there: it is never taken for either.
        client.send(place_rail(false));
        client.send(place_panel());
        let refused = connection
            .call_future(
                Some(BUS_NAME),
                OBJECT_PATH,
                INTERFACE,
                "Place",
                Some(&("main", 5i32, 5i32, true).to_variant()),
                None,
                gtk::gio::DBusCallFlags::NONE,
                5_000,
            )
            .await;
        if refused.is_ok() {
            return Err("a Place for a role that is not one was accepted".into());
        }

        // The rail and the panel come after, as the app makes them, and are
        // found by their markers.
        let rail_window = surface_window(application, Role::Rail);
        rail_window.set_decorated(false);
        rail_window.set_default_size(
            geometry::rail_size(false, 1048).0,
            geometry::rail_size(false, 1048).1,
        );
        let panel_window = surface_window(application, Role::Panel);
        panel_window.set_decorated(false);
        panel_window.set_default_size(geometry::PANEL_WIDTH, 400);
        rail_window.present();
        panel_window.present();
        let windows = Windows {
            main: main.clone(),
            rail: path_of(application, &rail_window),
            panel: path_of(application, &panel_window),
        };
        let result = steps(
            connection,
            client,
            &windows,
            &main_seen,
            &rail_window,
            &panel_window,
        )
        .await;
        rail_window.destroy();
        panel_window.destroy();
        result
    }

    async fn steps(
        connection: &gtk::gio::DBusConnection,
        client: &Client,
        windows: &Windows,
        main_seen: &Seen,
        rail: &gtk::ApplicationWindow,
        panel: &gtk::ApplicationWindow,
    ) -> Result<(), String> {
        let place_rail = |open| Request::Place {
            role: Role::Rail,
            at: (-1, -1),
            open,
        };
        let place_panel = || Request::Place {
            role: Role::Panel,
            at: PANEL_AT,
            open: true,
        };
        let Windows {
            main,
            rail: rail_path,
            panel: panel_path,
        } = windows;
        let (main, rail_path, panel_path) =
            (main.as_str(), rail_path.as_str(), panel_path.as_str());

        let shell = until(
            connection,
            "rail at the right edge, centred, above, on every workspace",
            |shell| {
                rail_ok(shell, rail_path)
                    && window(shell, rail_path).is_some_and(|w| w.above && w.sticky)
            },
        )
        .await?;
        for seen in [rail_path, panel_path].map(|path| window(&shell, path)) {
            let seen = seen.ok_or("a placed window")?;
            if seen.title != crate::copy::APP_NAME {
                return Err(format!("not titled with the app's name: {seen:?}"));
            }
        }
        let shell = until(connection, "panel near the press, above", |shell| {
            panel_ok(shell, panel_path, PANEL_AT)
                && window(shell, panel_path).is_some_and(|w| w.above)
        })
        .await?;
        main_untouched(&shell, main_seen)?;

        // The rail opens to the Mac's open size and is held again.
        rail.set_default_size(geometry::OPEN_WIDTH, 680);
        until(
            connection,
            "open rail at the right edge, centred",
            |shell| {
                window(shell, rail_path).is_some_and(|w| w.width == geometry::OPEN_WIDTH)
                    && rail_ok(shell, rail_path)
            },
        )
        .await?;

        // Taller than the work area: held to it (the Mac's min).
        rail.set_default_size(geometry::OPEN_WIDTH, 4000);
        until(connection, "tall rail held to the work area", |shell| {
            window(shell, rail_path).is_some_and(|w| w.height > 680) && rail_ok(shell, rail_path)
        })
        .await?;
        rail.set_default_size(geometry::OPEN_WIDTH, 680);
        let shell = until(connection, "rail back to the open size", |shell| {
            window(shell, rail_path).is_some_and(|w| w.height == 680) && rail_ok(shell, rail_path)
        })
        .await?;
        main_untouched(&shell, main_seen)?;

        // Focus (owner, 2026-11-18, "Keyboard only while open"). A click on
        // the closed rail leaves focus with the window before it: the Mac
        // panel never becomes key.
        with_windows(
            connection,
            &format!("w('{main}').activate(now); return '';"),
        )
        .await?;
        until_focus(connection, main, "the main window focused", |f| {
            f.focused == main
        })
        .await?;
        with_windows(
            connection,
            &format!("w('{rail_path}').activate(now); return '';"),
        )
        .await?;
        // The rail takes focus for a moment and the extension hands it back;
        // wait for it to be back, then make sure it stays.
        until_focus(
            connection,
            main,
            "closed rail: focus back on the main window",
            |f| f.focused == main,
        )
        .await?;
        glib::timeout_future(Duration::from_millis(500)).await;
        let f = focus_state(connection, main).await?;
        if f.focused != main {
            return Err(format!("closed rail: focus moved on again: {f:?}"));
        }

        // The open rail keeps it, so Escape and Tab reach it ...
        client.send(place_rail(true));
        with_windows(
            connection,
            &format!("w('{rail_path}').activate(now); return '';"),
        )
        .await?;
        until_focus(connection, main, "open rail: focus on the rail", |f| {
            f.focused == rail_path
        })
        .await?;
        glib::timeout_future(Duration::from_millis(500)).await;
        let f = focus_state(connection, main).await?;
        if f.focused != rail_path {
            return Err(format!("open rail: focus not kept: {f:?}"));
        }
        // ... and closing it gives focus back.
        client.send(place_rail(false));
        until_focus(
            connection,
            main,
            "rail closed: focus back on the main window",
            |f| f.focused == main,
        )
        .await?;

        // The panel is shown on every workspace and would take focus in
        // the steps below; hidden, the rail is the only other window left.
        panel.set_visible(false);
        until(connection, "panel hidden", |shell| {
            window(shell, panel_path).is_none_or(|w| !w.shown)
        })
        .await?;

        // A minimised window is never brought back to take focus.
        with_windows(
            connection,
            &format!("w('{main}').minimize(); w('{rail_path}').activate(now); return '';"),
        )
        .await?;
        until_focus(
            connection,
            main,
            "minimised: focus neither on the rail nor back",
            |f| f.minimised && f.focused != main && f.focused != rail_path,
        )
        .await?;
        glib::timeout_future(Duration::from_millis(500)).await;
        let f = focus_state(connection, main).await?;
        if !f.minimised || f.focused == main || f.focused == rail_path {
            return Err(format!("minimised: {f:?}"));
        }

        // Focus is never given back across workspaces: the rail is the only
        // window on the empty one.
        with_windows(
            connection,
            &format!(
                "w('{main}').unminimize(); w('{main}').activate(now); \
                 const m = global.workspace_manager; \
                 if (m.get_n_workspaces() < 2) m.append_new_workspace(false, now); \
                 return '';"
            ),
        )
        .await?;
        until_focus(connection, main, "the main window back and focused", |f| {
            !f.minimised && f.focused == main
        })
        .await?;
        with_windows(
            connection,
            "global.workspace_manager.get_workspace_by_index(1).activate(now); return '';",
        )
        .await?;
        until_focus(connection, main, "on the second workspace", |f| {
            f.workspace == 1
        })
        .await?;
        with_windows(
            connection,
            &format!("w('{rail_path}').activate(now); return '';"),
        )
        .await?;
        until_focus(
            connection,
            main,
            "workspace: focus neither on the rail nor back",
            |f| f.workspace == 1 && f.focused != main && f.focused != rail_path && !f.minimised,
        )
        .await?;
        glib::timeout_future(Duration::from_millis(500)).await;
        let f = focus_state(connection, main).await?;
        if f.workspace != 1 || f.focused == main || f.focused == rail_path || f.minimised {
            return Err(format!("workspace switch: {f:?}"));
        }
        with_windows(
            connection,
            "global.workspace_manager.get_workspace_by_index(0).activate(now); return '';",
        )
        .await?;
        until_focus(connection, main, "back on the first workspace", |f| {
            f.workspace == 0
        })
        .await?;
        panel.present();
        client.send(place_panel());
        let shell = until(connection, "panel shown and placed again", |shell| {
            panel_ok(shell, panel_path, PANEL_AT)
        })
        .await?;
        // Focus moved through the main window above; its frame and state
        // are what the extension must never change.
        main_untouched(&shell, main_seen)?;

        // The lock screen's shape: disabled, everything it did is undone;
        // enabled again, this client asks again and both are held again.
        eval(
            connection,
            &format!(
                "(() => {{ Main.extensionManager.disableExtension('{UUID}'); return ''; }})()"
            ),
        )
        .await?;
        let shell = until(connection, "both released on disable", |shell| {
            window(shell, rail_path).is_some_and(|w| !w.above && !w.sticky)
                && window(shell, panel_path).is_some_and(|w| !w.above)
        })
        .await?;
        main_untouched(&shell, main_seen)?;

        // Move the rail away so a replay has something to undo.
        with_windows(
            connection,
            &format!("w('{rail_path}').move_frame(false, 100, 100); return '';"),
        )
        .await?;
        until(connection, "rail moved away", |shell| {
            window(shell, rail_path).is_some_and(|w| (w.x, w.y) == (100, 100))
        })
        .await?;
        eval(
            connection,
            &format!("(() => {{ Main.extensionManager.enableExtension('{UUID}'); return ''; }})()"),
        )
        .await?;
        let shell = until(connection, "both held again after re-enable", |shell| {
            rail_ok(shell, rail_path)
                && window(shell, rail_path).is_some_and(|w| w.above && w.sticky)
                && window(shell, panel_path).is_some_and(|w| w.above)
        })
        .await?;
        main_untouched(&shell, main_seen)?;

        // Released by the app: no longer kept above.
        client.send(Request::Release { role: Role::Rail });
        let shell = until(connection, "rail released by Release", |shell| {
            window(shell, rail_path).is_some_and(|w| !w.above)
        })
        .await?;
        main_untouched(&shell, main_seen)
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs GNOME Shell with the extension and unsafe mode; scripts/gnome-shell-extension-verify.sh"]
    fn a_running_shell_holds_the_rail_and_the_panel() {
        let context = glib::MainContext::default();
        let _owner = context.acquire().expect("main context");
        gtk::init().expect("a display");
        let connection =
            gtk::gio::bus_get_sync(gtk::gio::BusType::Session, None::<&gtk::gio::Cancellable>)
                .expect("session bus");
        let application = gtk::Application::new(
            Some("ai.tracecommons.Contributor.EdgeLiveTest"),
            gtk::gio::ApplicationFlags::NON_UNIQUE,
        );
        application
            .register(None::<&gtk::gio::Cancellable>)
            .expect("registered");
        // The app's main window, as far as the extension can see: an
        // application window of the same process and connection, titled
        // with the app's name, without a marker.
        let main = gtk::ApplicationWindow::builder()
            .application(&application)
            .title(crate::copy::APP_NAME)
            .default_width(300)
            .default_height(200)
            .build();
        main.present();

        let outcome: Rc<RefCell<Option<Result<(), String>>>> = Rc::new(RefCell::new(None));
        let finished = Rc::new(Cell::new(false));
        {
            let (outcome, finished, application, main) = (
                Rc::clone(&outcome),
                Rc::clone(&finished),
                application.clone(),
                main.clone(),
            );
            glib::spawn_future_local(async move {
                let result = match find().await {
                    Some(client) => run(&connection, &client, &application, &main).await,
                    None => Err("the client did not find the extension".into()),
                };
                *outcome.borrow_mut() = Some(result);
                finished.set(true);
            });
        }
        while !finished.get() {
            context.iteration(true);
        }
        main.destroy();
        if let Some(Err(failure)) = outcome.borrow_mut().take() {
            panic!("{failure}");
        }
    }
}
