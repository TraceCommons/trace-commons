// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Places the Linux shell's edge rail and menu-bar panel on GNOME, which has
// no layer-shell for an application to place its own windows with. The
// app (crates/trace-commons-contributor-gtk, src/edge/gnome.rs) draws both
// itself, as ordinary GTK windows; this extension only moves them, keeps
// them above other windows and shows them on every workspace. It draws
// nothing and holds no words of its own.
//
// One narrow interface on the session bus: Place(role, x, y, open) and
// Release(role). A window is matched by the calling connection -- its
// process id and its own unique bus name, both of which GTK tells Mutter --
// and by a hidden marker only the rail and the panel carry: a disabled
// action named for the role, which the extension reads back from the
// window's exported action group (placement.js, markerFor). Titles play no
// part: the rail, the panel and the main window are all titled with the
// app's name (owner, 2026-11-18). The main window carries no marker, so it
// is never touched, and a caller can only ever place its own windows.
// Without this extension the app falls back on its own: no rail, a plain
// panel window.
//
// Keyboard focus (owner, 2026-11-18, "Keyboard only while open"): the open
// rail keeps it once given it, so Escape and Tab work; a closed rail never
// keeps it (the Mac's is a non-activating panel). Focus goes back only to
// a window on the active workspace and not minimised, or else to nothing,
// so it never switches workspaces or brings a window back (placement.js,
// focusAfter). Both windows are placed again when the monitors or work
// areas change. Everything this does to a window is undone by Release, by the
// caller leaving the bus, and by disable() -- which GNOME Shell calls on
// every screen lock; the app asks again when the name returns
// (src/edge/gnome.rs).
//
// Written against GNOME Shell 46 (Ubuntu 24.04) and the ES module
// extension API that 45 introduced, which 47-51 keep. Every Meta call here
// (get_pid, get_gtk_unique_bus_name, get_gtk_window_object_path and their
// notify signals, get_frame_rect, move_frame, make_above, stick,
// get_work_area_for_monitor, get_monitor, located_on_workspace, focus, the
// minimized property, the unmanaging, size-changed and shown signals, and
// the display's unset_input_focus, get_current_time_roundtrip and
// window-created) is in mutter's public API in each of those versions.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {
    FocusKeeper, ROLE_PANEL, ROLE_RAIL, couldBeCallers, frameFor, isMarked, markerFor, monitorFor,
} from './placement.js';

const BUS_NAME = 'ai.tracecommons.Contributor.EdgeSurfaces';
const OBJECT_PATH = '/ai/tracecommons/Contributor/EdgeSurfaces';
const VERSION = 3;

const IFACE = `<node>
  <interface name="ai.tracecommons.Contributor.EdgeSurfaces1">
    <method name="Place">
      <arg type="s" direction="in" name="role"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="b" direction="in" name="open"/>
    </method>
    <method name="Release">
      <arg type="s" direction="in" name="role"/>
    </method>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;

/** The pid behind a bus name, from the bus itself. */
function callerPid(sender) {
    return new Promise((resolve, reject) => {
        Gio.DBus.session.call(
            'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
            'GetConnectionUnixProcessID', new GLib.Variant('(s)', [sender]),
            new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, -1, null,
            (connection, result) => {
                try {
                    resolve(connection.call_finish(result).deepUnpack()[0]);
                } catch (error) {
                    reject(error);
                }
            });
    });
}

/**
 * The actions a window exports, read from the caller's own connection at the
 * window's object path; null when it cannot be read.
 */
function exportedActions(sender, path) {
    return new Promise(resolve => {
        Gio.DBus.session.call(
            sender, path, 'org.gtk.Actions', 'List', null,
            new GLib.VariantType('(as)'), Gio.DBusCallFlags.NONE, 2000, null,
            (connection, result) => {
                try {
                    resolve(connection.call_finish(result).deepUnpack()[0]);
                } catch {
                    resolve(null);
                }
            });
    });
}

export default class EdgeSurfacesExtension extends Extension {
    enable() {
        // key: sender + '\n' + role ->
        //   {pid, sender, role, at, open, window, signals, made: {above, stuck}}
        this._placements = new Map();
        // sender -> bus watch id, so a caller's placements end with it.
        this._watches = new Map();
        // window -> signal ids, for a caller's window whose path or bus
        // name GTK has not told Mutter yet.
        this._waiting = new Map();
        this._focus = new FocusKeeper(global.display, {
            isRail: w => this._placementOf(w)?.role === ROLE_RAIL,
            isOpen: w => Boolean(this._placementOf(w)?.open),
            onActiveWorkspace: w => w.located_on_workspace(
                global.workspace_manager.get_active_workspace()),
            isMinimized: w => w.minimized,
        });
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);
        // A new window has no path or bus name yet: GTK sends them after
        // Mutter has made the window, so a caller's window is watched for
        // them.
        this._created = global.display.connect('window-created',
            (_display, window) => this._consider(window));
        this._displayIds = [
            global.display.connect('workareas-changed', () => this._placeAll()),
            global.display.connect('notify::focus-window', () => this._focus.onFocus()),
        ];
        this._monitorsId = Main.layoutManager.connect('monitors-changed', () => this._placeAll());
    }

    disable() {
        global.display.disconnect(this._created);
        this._created = 0;
        for (const id of this._displayIds)
            global.display.disconnect(id);
        this._displayIds = [];
        Main.layoutManager.disconnect(this._monitorsId);
        this._monitorsId = 0;
        for (const window of [...this._waiting.keys()])
            this._stopWaiting(window);
        this._waiting = null;
        for (const placement of this._placements.values())
            this._forget(placement);
        this._placements = null;
        for (const id of this._watches.values())
            Gio.bus_unwatch_name(id);
        this._watches = null;
        this._focus.forget();
        this._focus = null;
        Gio.bus_unown_name(this._nameId);
        this._nameId = 0;
        this._dbus.unexport();
        this._dbus = null;
    }

    get Version() {
        return VERSION;
    }

    PlaceAsync([role, x, y, open], invocation) {
        if (markerFor(role) === null) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.InvalidArgs', 'role');
            return;
        }
        const sender = invocation.get_sender();
        callerPid(sender).then(pid => {
            if (!this._placements) {
                invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', 'disabled');
                return;
            }
            const key = `${sender}\n${role}`;
            const at = x < 0 || y < 0 ? global.get_pointer().slice(0, 2) : [x, y];
            const old = this._placements.get(key);
            if (old) {
                // The rail opening or closing, or the panel asked for again:
                // held as it is, only placed anew.
                old.at = at;
                old.open = open;
                old.place?.();
                if (old.window && global.display.focus_window === old.window)
                    this._focus.onFocus();
                if (!old.window) {
                    for (const actor of global.get_window_actors())
                        this._consider(actor.meta_window);
                }
                invocation.return_value(null);
                return;
            }
            const placement = {
                pid, sender, role, at, open, window: null, signals: [],
                made: {above: false, stuck: false},
            };
            this._placements.set(key, placement);
            this._watch(sender);
            for (const actor of global.get_window_actors())
                this._consider(actor.meta_window);
            invocation.return_value(null);
        }).catch(() => invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', 'caller'));
    }

    ReleaseAsync([role], invocation) {
        const key = `${invocation.get_sender()}\n${role}`;
        const placement = this._placements?.get(key);
        if (placement) {
            this._forget(placement);
            this._placements.delete(key);
        }
        invocation.return_value(null);
    }

    /** The placements still waiting for a window, from `window`'s process. */
    _waitingFor(window) {
        const pid = window.get_pid();
        return [...this._placements?.values() ?? []]
            .filter(placement => !placement.window && placement.pid === pid);
    }

    /**
     * Attaches `window` to a waiting placement of its caller if it carries
     * that role's marker; a caller's window GTK has not described yet is
     * watched until it is.
     */
    _consider(window) {
        if (!this._placements || this._placementOf(window))
            return;
        const waiting = this._waitingFor(window);
        if (waiting.length === 0)
            return;
        if (!window.get_gtk_window_object_path() || !window.get_gtk_unique_bus_name()) {
            this._waitFor(window);
            return;
        }
        this._stopWaiting(window);
        for (const placement of waiting) {
            if (!couldBeCallers(window, placement.pid, placement.sender))
                continue;
            const path = window.get_gtk_window_object_path();
            exportedActions(placement.sender, path).then(names => {
                if (!this._placements || placement.window ||
                    this._placements.get(`${placement.sender}\n${placement.role}`) !== placement ||
                    this._placementOf(window) ||
                    !couldBeCallers(window, placement.pid, placement.sender) ||
                    window.get_gtk_window_object_path() !== path ||
                    !isMarked(placement.role, names))
                    return;
                this._attach(placement, window);
            });
        }
    }

    _waitFor(window) {
        if (this._waiting.has(window))
            return;
        const again = () => this._consider(window);
        // GTK sends the path while realizing, before the window is first
        // shown; shown is the fallback should the notify not come.
        this._waiting.set(window, [
            window.connect('notify::gtk-window-object-path', again),
            window.connect('notify::gtk-unique-bus-name', again),
            window.connect('shown', again),
            window.connect('unmanaging', () => this._stopWaiting(window)),
        ]);
    }

    _stopWaiting(window) {
        const ids = this._waiting?.get(window);
        if (!ids)
            return;
        for (const id of ids)
            window.disconnect(id);
        this._waiting.delete(window);
    }

    _attach(placement, window) {
        placement.window = window;
        if (!window.is_above()) {
            window.make_above();
            placement.made.above = true;
        }
        if (!window.is_on_all_workspaces()) {
            window.stick();
            placement.made.stuck = true;
        }
        const place = () => {
            let monitor = global.display.get_primary_monitor();
            if (placement.role === ROLE_PANEL) {
                // The monitor that was clicked, else the window's own.
                const clicked = monitorFor(Main.layoutManager.monitors, placement.at);
                monitor = clicked >= 0 ? clicked : window.get_monitor();
            }
            const area = window.get_work_area_for_monitor(monitor);
            const rect = window.get_frame_rect();
            const frame = frameFor(placement.role, area, rect, placement.at);
            if (frame.x !== rect.x || frame.y !== rect.y)
                window.move_frame(true, frame.x, frame.y);
        };
        placement.place = place;
        placement.signals = [
            window.connect('size-changed', place),
            // A window asked for as soon as it maps is often not shown yet,
            // and Mutter's own first placement then moves it again (Shell
            // 46: the rail stayed where Mutter cascaded it until it was
            // first opened). Placed again once shown, it stays put.
            window.connect('shown', place),
            window.connect('unmanaging', () => {
                placement.signals = [];
                placement.window = null;
                placement.place = null;
                placement.made = {above: false, stuck: false};
            }),
        ];
        place();
        // A window placed now may have taken focus before it was known
        // to be the rail, and been noted as the window to go back to.
        if (this._focus.last === window)
            this._focus.note(null);
        if (global.display.focus_window === window)
            this._focus.onFocus();
    }

    /** Stops placing a window and undoes what placing it did. */
    _forget(placement) {
        const window = placement.window;
        if (window) {
            for (const id of placement.signals)
                window.disconnect(id);
            if (placement.made.above && window.is_above())
                window.unmake_above();
            if (placement.made.stuck && window.is_on_all_workspaces())
                window.unstick();
        }
        placement.signals = [];
        placement.window = null;
        placement.place = null;
        placement.made = {above: false, stuck: false};
    }

    _placeAll() {
        for (const placement of this._placements?.values() ?? [])
            placement.place?.();
    }

    _placementOf(window) {
        for (const placement of this._placements?.values() ?? []) {
            if (placement.window === window)
                return placement;
        }
        return null;
    }

    /** A caller's placements end when it leaves the bus. */
    _watch(sender) {
        if (this._watches.has(sender))
            return;
        this._watches.set(sender, Gio.bus_watch_name_on_connection(
            Gio.DBus.session, sender, Gio.BusNameWatcherFlags.NONE, null, () => {
                for (const [key, placement] of [...this._placements ?? []]) {
                    if (placement.sender !== sender)
                        continue;
                    this._forget(placement);
                    this._placements.delete(key);
                }
                const id = this._watches?.get(sender);
                if (id) {
                    this._watches.delete(sender);
                    Gio.bus_unwatch_name(id);
                }
            }));
    }
}
