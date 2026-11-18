// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Where the extension puts each window, which windows it may touch, and
// where keyboard focus goes. No GNOME imports, so it runs under plain node
// as well (tests/placement.test.js; the crate's tests read its names, see
// src/edge/gnome.rs).

export const ROLE_RAIL = 'rail';
export const ROLE_PANEL = 'panel';
// The hidden marker each role's window carries: the name of a disabled
// action in the window's own action group, which GTK exports on the app's
// session-bus connection at the window's object path (org.gtk.Actions) and
// tells Mutter about (gtk_shell1, or the _GTK_* properties on X11). Titles
// say nothing: the rail, the panel and the main window are all titled with
// the app's name (owner, 2026-11-18). The main window carries no marker, so
// it is never matched. The app's names: src/edge/gnome.rs, Role::marker.
export const MARKER_RAIL = 'edge-rail';
export const MARKER_PANEL = 'menu-panel';
// The panel's distance from the screen's edges: the app's PANEL_MARGIN.
export const MARGIN = 8;

/** Where a placement puts its window, in the monitor's work area. */
export function frameFor(role, area, size, at) {
    if (role === ROLE_RAIL) {
        return {
            x: area.x + area.width - size.width,
            y: area.y + Math.floor((area.height - size.height) / 2),
        };
    }
    const [px, py] = at;
    const left = Math.min(
        Math.max(px - Math.floor(size.width / 2), area.x + MARGIN),
        Math.max(area.x + MARGIN, area.x + area.width - size.width - MARGIN));
    const top = py < area.y + area.height / 2
        ? area.y + MARGIN
        : area.y + area.height - size.height - MARGIN;
    return {x: left, y: top};
}

/** The marker a role's window carries, or null for a role that is not one. */
export function markerFor(role) {
    if (role === ROLE_RAIL)
        return MARKER_RAIL;
    if (role === ROLE_PANEL)
        return MARKER_PANEL;
    return null;
}

/**
 * Whether `window` could be one of the caller's: the caller's process, the
 * caller's own bus connection, and a window GTK exported an object path
 * for. Only such a window is then asked for its actions.
 */
export function couldBeCallers(window, pid, sender) {
    return window.get_pid() === pid && window.get_gtk_unique_bus_name() === sender &&
        Boolean(window.get_gtk_window_object_path());
}

/** Whether a window exporting the actions `names` carries `role`'s marker. */
export function isMarked(role, names) {
    const marker = markerFor(role);
    return marker !== null && Array.isArray(names) && names.includes(marker);
}

/**
 * The index of the monitor holding `at`, from `monitors` ({x, y, width,
 * height} each, in layout order), or -1 when it is on none.
 */
export function monitorFor(monitors, at) {
    const [x, y] = at;
    return monitors.findIndex(m =>
        x >= m.x && x < m.x + m.width && y >= m.y && y < m.y + m.height);
}

export const FOCUS_LEAVE = 'leave';
export const FOCUS_RESTORE = 'restore';
export const FOCUS_UNSET = 'unset';

/**
 * What to do with keyboard focus once it lands on `focused` (owner,
 * 2026-11-18, "Keyboard only while open"): an open rail keeps it, so
 * Escape and Tab reach it; a closed rail never does -- the Mac's rail is a
 * non-activating panel that "never takes focus from the app in front".
 * Focus goes back only to a window still managed, on the active workspace
 * and not minimised, so giving it back can never switch workspaces or
 * bring a minimised window back; otherwise nothing has focus.
 *
 * `shell` answers isRail, isOpen, isManaged, onActiveWorkspace and
 * isMinimized for a window. Returns {action, window}, window set only for
 * FOCUS_RESTORE.
 */
export function focusAfter(focused, previous, shell) {
    const leave = {action: FOCUS_LEAVE, window: null};
    if (!focused || !shell.isRail(focused) || shell.isOpen(focused))
        return leave;
    const usable = previous && previous !== focused && !shell.isRail(previous) &&
        shell.isManaged(previous) && shell.onActiveWorkspace(previous) &&
        !shell.isMinimized(previous);
    return usable
        ? {action: FOCUS_RESTORE, window: previous}
        : {action: FOCUS_UNSET, window: null};
}

/**
 * Applies focusAfter as focus moves. `display` is global.display (or a
 * stand-in): focus_window, get_current_time_roundtrip, unset_input_focus.
 * `shell` answers isRail, isOpen, onActiveWorkspace and isMinimized; this
 * keeper answers isManaged itself: the window noted last is managed until
 * it sends unmanaging (not unmanaged: focus moves on while a window is
 * going, before unmanaged is sent), and is then let go of.
 */
export class FocusKeeper {
    constructor(display, shell) {
        this._display = display;
        this._shell = {...shell, isManaged: w => this.isManaged(w)};
        this._last = null;
        this._lastId = 0;
        this._going = new WeakSet();
    }

    isManaged(window) {
        return !this._going.has(window);
    }

    /** The window focus would go back to, if any. */
    get last() {
        return this._last;
    }

    /** Called whenever the display's focus window changes. */
    onFocus() {
        const focused = this._display.focus_window;
        const decision = focusAfter(focused, this._last, this._shell);
        if (decision.action === FOCUS_RESTORE) {
            // focus, not activate: it neither raises, nor switches
            // workspace, nor unminimises.
            decision.window.focus(this._display.get_current_time_roundtrip());
            return;
        }
        if (decision.action === FOCUS_UNSET) {
            this._display.unset_input_focus(this._display.get_current_time_roundtrip());
            return;
        }
        if (focused && !this._shell.isRail(focused))
            this.note(focused);
    }

    /** Notes `window` as the one focus goes back to (null for none). */
    note(window) {
        if (this._last && this._lastId)
            this._last.disconnect(this._lastId);
        this._last = window;
        this._lastId = window
            ? window.connect('unmanaging', going => {
                this._going.add(going);
                this.note(null);
            })
            : 0;
    }

    /** Forgets the noted window, as the extension is disabled. */
    forget() {
        this.note(null);
    }
}
