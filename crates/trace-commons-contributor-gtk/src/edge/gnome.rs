//! The GNOME Shell extension's side of the bus, as this app sees it.
//!
//! GNOME has no layer-shell, and a GTK 4 window cannot place itself. The
//! extension in `gnome-shell-extension/` does it for this app's two
//! windows: it owns [`BUS_NAME`] on the session bus and exports one narrow
//! interface, [`INTERFACE`], at [`OBJECT_PATH`]:
//!
//! * `Place(s title, s role, i x, i y)`: the calling process's window with
//!   that title is put where `role` says -- `"rail"` against the primary
//!   monitor's right edge, centred, and kept there as it resizes;
//!   `"panel"` near (`x`, `y`), or near the pointer when they are
//!   negative -- kept above other windows and shown on every workspace.
//! * `Release(s title)`: stop placing it.
//! * `Version` (u): the interface's version, [`VERSION`].
//!
//! The extension matches windows by the caller's process id and the title,
//! so it cannot be pointed at another application's windows. Nothing else
//! crosses: no trace, no setting, no word. Without the extension the app
//! works as before (`capability`): no rail, and a plain panel window.
//!
//! GNOME Shell disables every extension on the lock screen and enables it
//! again on unlock, and the extension undoes its placements when disabled.
//! So the client remembers the last placement asked for each window and
//! asks again whenever the extension's name gets a new owner.

/// The extension's well-known name. Under the app's own id, so a Flatpak
/// sandbox may talk to it without a further hole.
pub const BUS_NAME: &str = "ai.tracecommons.Contributor.EdgeSurfaces";
pub const OBJECT_PATH: &str = "/ai/tracecommons/Contributor/EdgeSurfaces";
pub const INTERFACE: &str = "ai.tracecommons.Contributor.EdgeSurfaces1";
pub const VERSION: u32 = 1;

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
}

/// One request to the extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Place {
        title: String,
        role: Role,
        /// Where the panel was asked for; negative for the pointer's
        /// position. Ignored for the rail.
        at: (i32, i32),
    },
    Release {
        title: String,
    },
}

/// The extension's answer to "are you there", and the way to ask it for
/// placements.
#[derive(Clone)]
pub struct Client {
    tx: async_channel::Sender<Message>,
}

impl Client {
    pub fn send(&self, request: Request) {
        let _ = self.tx.send_blocking(Message::Request(request));
    }
}

/// What the client's thread acts on: a request from the app, or the
/// extension's name gaining an owner again.
enum Message {
    Request(Request),
    Returned,
}

/// The placements to ask for again, by window title: the last `Place` for
/// each, gone once released.
#[derive(Default, Debug, PartialEq, Eq)]
struct Held {
    places: std::collections::BTreeMap<String, Request>,
}

impl Held {
    fn record(&mut self, request: &Request) {
        match request {
            Request::Place { title, .. } => {
                self.places.insert(title.clone(), request.clone());
            }
            Request::Release { title } => {
                self.places.remove(title);
            }
        }
    }

    fn replay(&self) -> impl Iterator<Item = &Request> {
        self.places.values()
    }
}

/// Starts the client on its own thread. The receiver answers once: whether
/// the extension owns its name. A session with no bus answers false.
pub fn spawn() -> (Client, async_channel::Receiver<bool>) {
    let (tx, requests) = async_channel::unbounded::<Message>();
    let (present_tx, present) = async_channel::bounded(1);
    let returned = tx.clone();
    std::thread::spawn(move || {
        let outcome = crate::dbus::with_runtime(|| run(requests, returned, present_tx.clone()));
        if outcome.is_err() {
            let _ = present_tx.send_blocking(false);
        }
    });
    (Client { tx }, present)
}

fn run(
    requests: async_channel::Receiver<Message>,
    returned: async_channel::Sender<Message>,
    present: async_channel::Sender<bool>,
) -> anyhow::Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    let bus = zbus::blocking::fdo::DBusProxy::new(&connection)?;
    let name = zbus::names::BusName::try_from(BUS_NAME)?;
    let owned = bus.name_has_owner(name).unwrap_or(false);
    let _ = present.send_blocking(owned);
    if !owned {
        return Ok(());
    }
    // The name gains a new owner each time the extension is enabled again
    // (after every screen unlock). Watched on its own thread, which only
    // tells this one; a failure to watch costs the replay, nothing else.
    if let Ok(changes) = bus.receive_name_owner_changed_with_args(&[(0, BUS_NAME)]) {
        std::thread::spawn(move || {
            for change in changes {
                let back = change.args().is_ok_and(|args| args.new_owner().is_some());
                if back && returned.send_blocking(Message::Returned).is_err() {
                    return;
                }
            }
        });
    }
    let proxy = zbus::blocking::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE)?;
    let mut held = Held::default();
    while let Ok(message) = requests.recv_blocking() {
        match message {
            Message::Request(request) => {
                held.record(&request);
                call(&proxy, &request);
            }
            Message::Returned => {
                for request in held.replay() {
                    call(&proxy, request);
                }
            }
        }
    }
    Ok(())
}

/// One call. A failed call is dropped: the window stays where the window
/// manager put it, which is the no-extension behaviour.
fn call(proxy: &zbus::blocking::Proxy<'_>, request: &Request) {
    let _ = match request {
        Request::Place { title, role, at } => {
            proxy.call::<_, _, ()>("Place", &(title.as_str(), role.wire(), at.0, at.1))
        }
        Request::Release { title } => proxy.call::<_, _, ()>("Release", &(title.as_str(),)),
    };
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
            "<method name=\"Release\">".to_string(),
        ] {
            assert!(extension.contains(&needle), "extension.js lacks {needle}");
        }
        let placement = extension_file("placement.js");
        for needle in [
            format!("ROLE_RAIL = '{}';", Role::Rail.wire()),
            format!("ROLE_PANEL = '{}';", Role::Panel.wire()),
            format!("MARGIN = {};", crate::edge::geometry::PANEL_MARGIN),
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
        let place = |title: &str, role| Request::Place {
            title: title.into(),
            role,
            at: (-1, -1),
        };
        let mut held = Held::default();
        held.record(&place("rail", Role::Rail));
        held.record(&place("panel", Role::Panel));
        held.record(&Request::Place {
            title: "panel".into(),
            role: Role::Panel,
            at: (5, 6),
        });
        held.record(&Request::Release {
            title: "rail".into(),
        });
        let again: Vec<&Request> = held.replay().collect();
        assert_eq!(
            again,
            vec![&Request::Place {
                title: "panel".into(),
                role: Role::Panel,
                at: (5, 6),
            }],
            "the last placement per window, and nothing released"
        );
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
/// The expected frames are restated here, independently of `placement.js`,
/// so a wrong port there shows up as a disagreement: the rail by the Mac's
/// `EdgeRailGeometry.frame` (right work-area edge, centred, no taller than
/// the work area), the panel by `geometry::panel_anchor`'s rule.
#[cfg(test)]
#[cfg(target_os = "linux")]
mod live_tests {

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use gtk::glib;
    use gtk::prelude::*;

    use super::*;
    use crate::edge::geometry;

    const UUID: &str = "edge-surfaces@tracecommons.ai";

    /// One of this process's windows as Mutter sees it.
    #[derive(Debug, serde::Deserialize)]
    struct Seen {
        title: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        above: bool,
        sticky: bool,
        focused: bool,
    }

    #[derive(Debug, serde::Deserialize)]
    struct Shell {
        work_area: [i32; 4],
        windows: Vec<Seen>,
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

    async fn look(connection: &gtk::gio::DBusConnection) -> Result<Shell, String> {
        let js = format!(
            "(() => {{ const m = global.display.get_primary_monitor(); \
             const wa = global.workspace_manager.get_active_workspace().get_work_area_for_monitor(m); \
             const f = global.display.focus_window; \
             return JSON.stringify({{ work_area: [wa.x, wa.y, wa.width, wa.height], \
               windows: global.get_window_actors().map(a => a.meta_window) \
                 .filter(w => w.get_pid() === {pid}) \
                 .map(w => {{ const r = w.get_frame_rect(); return {{ \
                   title: w.get_title() || '', x: r.x, y: r.y, width: r.width, height: r.height, \
                   above: w.is_above(), sticky: w.is_on_all_workspaces(), focused: w === f }}; }}) }}); }})()",
            pid = std::process::id(),
        );
        // Eval answers with the value JSON-encoded, and the value is itself a
        // JSON string.
        let text = eval(connection, &js).await?;
        let inner: String = serde_json::from_str(&text).map_err(|e| format!("{e}: {text}"))?;
        serde_json::from_str(&inner).map_err(|e| format!("{e}: {inner}"))
    }

    fn window<'a>(shell: &'a Shell, title: &str) -> Result<&'a Seen, String> {
        shell
            .windows
            .iter()
            .find(|w| w.title == title)
            .ok_or_else(|| format!("no window titled {title}: {shell:?}"))
    }

    fn check(condition: bool, what: &str, shell: &Shell) -> Result<(), String> {
        if condition {
            Ok(())
        } else {
            Err(format!("{what}: {shell:?}"))
        }
    }

    /// The Mac's rail frame: against the right edge, centred, never taller.
    fn rail_ok(shell: &Shell, rail: &Seen) -> bool {
        let [x, y, w, h] = shell.work_area;
        rail.height <= h
            && rail.x == x + w - rail.width
            && rail.y == y + (h - rail.height).div_euclid(2)
    }

    /// `geometry::panel_anchor`'s rule for a press at `at`: centred on it
    /// across, inside the margin, and against the top or bottom margin by the
    /// half it is in.
    fn panel_expected(shell: &Shell, panel: &Seen, at: (i32, i32)) -> (i32, i32) {
        let [x, y, w, h] = shell.work_area;
        let margin = geometry::PANEL_MARGIN;
        let left = (at.0 - panel.width / 2)
            .max(x + margin)
            .min((x + margin).max(x + w - panel.width - margin));
        let top = if at.1 * 2 < 2 * y + h {
            y + margin
        } else {
            y + h - panel.height - margin
        };
        (left, top)
    }

    async fn settle() {
        glib::timeout_future(Duration::from_millis(1500)).await;
    }

    const RAIL: &str = "rail-under-test";
    const PANEL: &str = "panel-under-test";
    const PANEL_AT: (i32, i32) = (1700, 10);

    async fn run(
        connection: &gtk::gio::DBusConnection,
        client: &Client,
        present: bool,
        rail: &gtk::Window,
    ) -> Result<(), String> {
        if !present {
            return Err("the client did not find the extension".into());
        }
        let place_rail = || Request::Place {
            title: RAIL.into(),
            role: Role::Rail,
            at: (-1, -1),
        };
        let place_panel = || Request::Place {
            title: PANEL.into(),
            role: Role::Panel,
            at: PANEL_AT,
        };
        client.send(place_rail());
        client.send(place_panel());
        settle().await;

        let shell = look(connection).await?;
        let r = window(&shell, RAIL)?;
        check(
            rail_ok(&shell, r),
            "rail at the right edge, centred",
            &shell,
        )?;
        check(
            r.above && r.sticky,
            "rail above, on every workspace",
            &shell,
        )?;
        let p = window(&shell, PANEL)?;
        check(
            (p.x, p.y) == panel_expected(&shell, p, PANEL_AT),
            "panel near the press",
            &shell,
        )?;
        check(p.above, "panel above", &shell)?;

        // The rail opens to the Mac's open size and is held again.
        rail.set_default_size(geometry::OPEN_WIDTH, 680);
        settle().await;
        let shell = look(connection).await?;
        let r = window(&shell, RAIL)?;
        check(r.width == geometry::OPEN_WIDTH, "rail grew", &shell)?;
        check(
            rail_ok(&shell, r),
            "open rail at the right edge, centred",
            &shell,
        )?;

        // Taller than the work area: held to it (the Mac's min).
        rail.set_default_size(geometry::OPEN_WIDTH, 4000);
        settle().await;
        let shell = look(connection).await?;
        let r = window(&shell, RAIL)?;
        check(
            rail_ok(&shell, r),
            "tall rail held to the work area",
            &shell,
        )?;
        rail.set_default_size(geometry::OPEN_WIDTH, 680);

        // A click on the rail leaves focus with the window before it: the Mac
        // panel never becomes key.
        eval(
            connection,
            &format!(
                "(() => {{ const ws = global.get_window_actors().map(a => a.meta_window) \
                 .filter(w => w.get_pid() === {pid}); \
                 const p = ws.find(w => w.get_title() === '{PANEL}'); \
                 const r = ws.find(w => w.get_title() === '{RAIL}'); \
                 p.activate(global.get_current_time()); r.activate(global.get_current_time()); \
                 return ''; }})()",
                pid = std::process::id(),
            ),
        )
        .await?;
        settle().await;
        let shell = look(connection).await?;
        check(
            window(&shell, PANEL)?.focused,
            "focus back on the panel",
            &shell,
        )?;

        // The lock screen's shape: disabled, everything it did is undone;
        // enabled again, this client asks again and both are held again.
        eval(
            connection,
            &format!(
                "(() => {{ Main.extensionManager.disableExtension('{UUID}'); return ''; }})()"
            ),
        )
        .await?;
        settle().await;
        let shell = look(connection).await?;
        let r = window(&shell, RAIL)?;
        check(!r.above && !r.sticky, "rail released on disable", &shell)?;
        check(
            !window(&shell, PANEL)?.above,
            "panel released on disable",
            &shell,
        )?;

        // Move the rail away so a replay has something to undo.
        eval(
            connection,
            &format!(
                "(() => {{ global.get_window_actors().map(a => a.meta_window) \
                 .find(w => w.get_pid() === {pid} && w.get_title() === '{RAIL}') \
                 .move_frame(false, 100, 100); return ''; }})()",
                pid = std::process::id(),
            ),
        )
        .await?;
        eval(
            connection,
            &format!("(() => {{ Main.extensionManager.enableExtension('{UUID}'); return ''; }})()"),
        )
        .await?;
        settle().await;
        let shell = look(connection).await?;
        let r = window(&shell, RAIL)?;
        check(
            rail_ok(&shell, r) && r.above && r.sticky,
            "rail held again after re-enable",
            &shell,
        )?;
        check(
            window(&shell, PANEL)?.above,
            "panel held again after re-enable",
            &shell,
        )?;

        // Released by the app: no longer kept above.
        client.send(Request::Release { title: RAIL.into() });
        settle().await;
        let shell = look(connection).await?;
        check(
            !window(&shell, RAIL)?.above,
            "rail released by Release",
            &shell,
        )
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

        let rail = gtk::Window::builder()
            .title(RAIL)
            .decorated(false)
            .default_width(geometry::rail_size(false, 1048).0)
            .default_height(geometry::rail_size(false, 1048).1)
            .build();
        let panel = gtk::Window::builder()
            .title(PANEL)
            .decorated(false)
            .default_width(geometry::PANEL_WIDTH)
            .default_height(400)
            .build();
        rail.present();
        panel.present();

        let (client, present) = spawn();
        let outcome: Rc<RefCell<Option<Result<(), String>>>> = Rc::new(RefCell::new(None));
        let finished = Rc::new(Cell::new(false));
        {
            let (outcome, finished, rail) =
                (Rc::clone(&outcome), Rc::clone(&finished), rail.clone());
            glib::spawn_future_local(async move {
                let present = present.recv().await.unwrap_or(false);
                let result = run(&connection, &client, present, &rail).await;
                *outcome.borrow_mut() = Some(result);
                finished.set(true);
            });
        }
        while !finished.get() {
            context.iteration(true);
        }
        drop((rail, panel));
        if let Some(Err(failure)) = outcome.borrow_mut().take() {
            panic!("{failure}");
        }
    }
}
