// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Places the Linux shell's edge rail and menu-bar panel on GNOME, which has
// no layer-shell for an application to place its own windows with. The
// app (crates/trace-commons-contributor-gtk, src/edge/gnome.rs) draws both
// itself, as ordinary GTK windows; this extension only moves them, keeps
// them above other windows and shows them on every workspace. It draws
// nothing and holds no words of its own.
//
// One narrow interface on the session bus: Place(title, role, x, y) and
// Release(title). A window is matched by the calling process's id and its
// title, so a caller can only ever place its own windows. Without this
// extension the app falls back on its own: no rail, a plain panel window.
//
// The rail never keeps keyboard focus (the Mac's is a non-activating
// panel): when a click gives it focus, focus goes back to the window that
// had it. Both windows are placed again when the monitors or work areas
// change. Everything this does to a window is undone by Release, by the
// caller leaving the bus, and by disable() -- which GNOME Shell calls on
// every screen lock; the app asks again when the name returns
// (src/edge/gnome.rs).
//
// Written against GNOME Shell 46 (Ubuntu 24.04) and the ES module
// extension API that 45 introduced, which 47-51 keep. Every Meta call here
// (get_pid, get_title, get_gtk_window_object_path, get_frame_rect,
// move_frame, make_above, stick, get_work_area_for_monitor) and the display's window-created signal are
// present in each of those versions.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {ROLE_PANEL, ROLE_RAIL, focusToRestore, frameFor} from './placement.js';

const BUS_NAME = 'ai.tracecommons.Contributor.EdgeSurfaces';
const OBJECT_PATH = '/ai/tracecommons/Contributor/EdgeSurfaces';
const VERSION = 1;

const IFACE = `<node>
  <interface name="ai.tracecommons.Contributor.EdgeSurfaces1">
    <method name="Place">
      <arg type="s" direction="in" name="title"/>
      <arg type="s" direction="in" name="role"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
    </method>
    <method name="Release">
      <arg type="s" direction="in" name="title"/>
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
 * Whether `window` is the caller's window of that title. The app's main
 * window may share the panel's title, but it is a GtkApplicationWindow,
 * which GTK names with an object path (gtk-shell on Wayland, a window
 * property on X11); the two windows placed here are plain GtkWindows and
 * have none.
 */
function matches(window, pid, title) {
    return window.get_pid() === pid && window.get_title() === title &&
        !window.get_gtk_window_object_path();
}

export default class EdgeSurfacesExtension extends Extension {
    enable() {
        // key: pid + '\n' + title ->
        //   {pid, sender, title, role, at, window, signals, made: {above, stuck}}
        this._placements = new Map();
        // sender -> bus watch id, so a caller's placements end with it.
        this._watches = new Map();
        this._lastFocus = null;
        this._lastFocusId = 0;
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);
        this._created = global.display.connect('window-created', (_display, window) => {
            // The title may arrive a moment after the window does.
            GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                this._adopt(window);
                return GLib.SOURCE_REMOVE;
            });
        });
        this._displayIds = [
            global.display.connect('workareas-changed', () => this._placeAll()),
            global.display.connect('notify::focus-window', () => this._onFocus()),
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
        for (const placement of this._placements.values())
            this._forget(placement);
        this._placements = null;
        for (const id of this._watches.values())
            Gio.bus_unwatch_name(id);
        this._watches = null;
        this._setLastFocus(null);
        Gio.bus_unown_name(this._nameId);
        this._nameId = 0;
        this._dbus.unexport();
        this._dbus = null;
    }

    get Version() {
        return VERSION;
    }

    PlaceAsync([title, role, x, y], invocation) {
        if (role !== ROLE_RAIL && role !== ROLE_PANEL) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.InvalidArgs', 'role');
            return;
        }
        const sender = invocation.get_sender();
        callerPid(sender).then(pid => {
            if (!this._placements) {
                invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', 'disabled');
                return;
            }
            const key = `${pid}\n${title}`;
            const old = this._placements.get(key);
            if (old)
                this._forget(old);
            const at = x < 0 || y < 0 ? global.get_pointer().slice(0, 2) : [x, y];
            const placement = {
                pid, sender, title, role, at, window: null, signals: [],
                made: {above: false, stuck: false},
            };
            this._placements.set(key, placement);
            this._watch(sender);
            const window = global.get_window_actors()
                .map(actor => actor.meta_window)
                .find(candidate => matches(candidate, pid, title));
            if (window)
                this._attach(placement, window);
            invocation.return_value(null);
        }).catch(() => invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', 'caller'));
    }

    ReleaseAsync([title], invocation) {
        callerPid(invocation.get_sender()).then(pid => {
            const key = `${pid}\n${title}`;
            const placement = this._placements?.get(key);
            if (placement) {
                this._forget(placement);
                this._placements.delete(key);
            }
            invocation.return_value(null);
        }).catch(() => invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed', 'caller'));
    }

    _adopt(window) {
        if (!this._placements)
            return;
        const placement = this._placements.get(`${window.get_pid()}\n${window.get_title()}`);
        if (placement && !placement.window && matches(window, placement.pid, placement.title))
            this._attach(placement, window);
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
            const monitor = placement.role === ROLE_RAIL
                ? global.display.get_primary_monitor()
                : window.get_monitor();
            const area = window.get_work_area_for_monitor(monitor);
            const rect = window.get_frame_rect();
            const frame = frameFor(placement.role, area, rect, placement.at);
            if (frame.x !== rect.x || frame.y !== rect.y)
                window.move_frame(true, frame.x, frame.y);
        };
        placement.place = place;
        placement.signals = [
            window.connect('size-changed', place),
            window.connect('unmanaged', () => {
                placement.signals = [];
                placement.window = null;
                placement.place = null;
                placement.made = {above: false, stuck: false};
            }),
        ];
        place();
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

    _isRail(window) {
        for (const placement of this._placements?.values() ?? []) {
            if (placement.window === window)
                return placement.role === ROLE_RAIL;
        }
        return false;
    }

    _onFocus() {
        const focused = global.display.focus_window;
        const back = focusToRestore(focused, this._lastFocus, w => this._isRail(w));
        if (back) {
            back.activate(global.get_current_time());
            return;
        }
        if (focused && !this._isRail(focused))
            this._setLastFocus(focused);
    }

    _setLastFocus(window) {
        if (this._lastFocus && this._lastFocusId)
            this._lastFocus.disconnect(this._lastFocusId);
        this._lastFocus = window;
        this._lastFocusId = window
            ? window.connect('unmanaged', () => this._setLastFocus(null))
            : 0;
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
