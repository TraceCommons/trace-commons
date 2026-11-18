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

/// The way to ask the extension for placements, once it is known to be
/// there. Calls go over GIO's session-bus connection on the main loop; a
/// failed call is dropped, and the window stays where the window manager
/// put it, which is the no-extension behaviour.
#[derive(Clone)]
pub struct Client {
    connection: gtk::gio::DBusConnection,
}

impl Client {
    pub fn send(&self, request: Request) {
        use gtk::glib::variant::ToVariant;
        let (method, args) = match request {
            Request::Place { title, role, at } => (
                "Place",
                (title, role.wire().to_string(), at.0, at.1).to_variant(),
            ),
            Request::Release { title } => ("Release", (title,).to_variant()),
        };
        self.connection.call(
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
    owned.then_some(Client { connection })
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
    fn the_bus_name_sits_under_the_apps_id() {
        assert!(BUS_NAME.starts_with(&format!("{}.", crate::ui::APP_ID)));
        assert_eq!(Role::Rail.wire(), "rail");
        assert_eq!(Role::Panel.wire(), "panel");
    }
}
