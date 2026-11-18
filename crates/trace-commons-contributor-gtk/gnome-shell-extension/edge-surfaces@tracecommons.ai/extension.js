// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Places the Linux shell's edge rail and menu-bar panel on GNOME, which has
// no layer-shell for an application to place its own windows with. The
// app (crates/trace-commons-contributor-gtk, src/edge/gnome.rs) draws both
// itself, as ordinary GTK windows; this extension only moves them, keeps
// them above other windows and shows them on every workspace. It draws
// nothing and holds no words of its own.
//
// One narrow interface on the session bus: Place(title, role, x, y, open)
// and Release(title). A window is matched by the calling process's id and
// its title, and each role places only the one title placement.js names
// for it, which no other window of the app carries: a caller can only ever
// place its own rail and panel, never its main window or anyone else's.
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
// (get_pid, get_title, get_gtk_window_object_path, get_frame_rect,
// move_frame, make_above, stick, get_work_area_for_monitor,
// located_on_workspace, focus, the minimized property, the unmanaging
// signal, and the display's unset_input_focus,
// get_current_time_roundtrip and window-created) is in mutter's public
// API in each of those versions.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {
    FOCUS_RESTORE, FOCUS_UNSET, ROLE_RAIL, focusAfter, frameFor, titleFor,
} from './placement.js';

const BUS_NAME = 'ai.tracecommons.Contributor.EdgeSurfaces';
const OBJECT_PATH = '/ai/tracecommons/Contributor/EdgeSurfaces';
const VERSION = 2;

const IFACE = `<node>
  <interface name="ai.tracecommons.Contributor.EdgeSurfaces1">
    <method name="Place">
      <arg type="s" direction="in" name="title"/>
      <arg type="s" direction="in" name="role"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="b" direction="in" name="open"/>
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
 * Whether `window` is the caller's window of that title. The title is one
 * only the rail or the panel carries (PlaceAsync refuses any other). As a
 * second guard, a GtkApplicationWindow -- the app's main window is one, and
 * GTK names it with an object path where it can -- is never matched; the
 * two windows placed here are plain GtkWindows.
 */
function matches(window, pid, title) {
    return window.get_pid() === pid && window.get_title() === title &&
        !window.get_gtk_window_object_path();
}

export default class EdgeSurfacesExtension extends Extension {
    enable() {
        // key: pid + '\n' + title ->
        //   {pid, sender, title, role, at, open, window, signals, made: {above, stuck}}
        this._placements = new Map();
        // sender -> bus watch id, so a caller's placements end with it.
        this._watches = new Map();
        this._lastFocus = null;
        this._lastFocusId = 0;
        // The idle callbacks window-created queued, removed on disable.
        this._idles = new Set();
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);
        this._created = global.display.connect('window-created', (_display, window) => {
            // The title may arrive a moment after the window does.
            const id = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                this._idles?.delete(id);
                this._adopt(window);
                return GLib.SOURCE_REMOVE;
            });
            this._idles.add(id);
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
        for (const id of this._idles)
            GLib.source_remove(id);
        this._idles = null;
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

    PlaceAsync([title, role, x, y, open], invocation) {
        if (titleFor(role) === null || titleFor(role) !== title) {
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
            const at = x < 0 || y < 0 ? global.get_pointer().slice(0, 2) : [x, y];
            const old = this._placements.get(key);
            if (old && old.window && old.sender === sender) {
                // The rail opening or closing, or the panel asked for again:
                // held as it is, only placed anew.
                old.at = at;
                old.open = open;
                old.place?.();
                if (global.display.focus_window === old.window)
                    this._onFocus();
                invocation.return_value(null);
                return;
            }
            if (old)
                this._forget(old);
            const placement = {
                pid, sender, title, role, at, open, window: null, signals: [],
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
        if (this._lastFocus === window)
            this._setLastFocus(null);
        if (global.display.focus_window === window)
            this._onFocus();
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

    _onFocus() {
        const focused = global.display.focus_window;
        const shell = {
            isRail: w => this._placementOf(w)?.role === ROLE_RAIL,
            isOpen: w => Boolean(this._placementOf(w)?.open),
            // The window noted last is forgotten as it starts to go.
            isManaged: w => w === this._lastFocus,
            onActiveWorkspace: w => w.located_on_workspace(
                global.workspace_manager.get_active_workspace()),
            isMinimized: w => w.minimized,
        };
        const decision = focusAfter(focused, this._lastFocus, shell);
        const now = global.display.get_current_time_roundtrip();
        if (decision.action === FOCUS_RESTORE) {
            // focus, not activate: it neither raises, nor switches
            // workspace, nor unminimises.
            decision.window.focus(now);
            return;
        }
        if (decision.action === FOCUS_UNSET) {
            global.display.unset_input_focus(now);
            return;
        }
        if (focused && !shell.isRail(focused))
            this._setLastFocus(focused);
    }

    _setLastFocus(window) {
        if (this._lastFocus && this._lastFocusId)
            this._lastFocus.disconnect(this._lastFocusId);
        this._lastFocus = window;
        // unmanaging, not unmanaged: focus moves on while a window is
        // going, before unmanaged is sent.
        this._lastFocusId = window
            ? window.connect('unmanaging', () => this._setLastFocus(null))
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
