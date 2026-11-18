// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Where the extension puts each window. No GNOME imports, so it runs
// under plain node as well (the crate's tests read its numbers; see
// src/edge/gnome.rs).

export const ROLE_RAIL = 'rail';
export const ROLE_PANEL = 'panel';
// The only window titles each role places: names no other window of the
// app carries, so the app's main window can never be matched, whatever
// GTK exports for it. The app gives its windows these titles only on GNOME
// with this extension (src/edge/gnome.rs, RAIL_TITLE and PANEL_TITLE).
export const TITLE_RAIL = 'edge-rail';
export const TITLE_PANEL = 'menu-panel';
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

/** The title a role places, or null for a role that is not one. */
export function titleFor(role) {
    if (role === ROLE_RAIL)
        return TITLE_RAIL;
    if (role === ROLE_PANEL)
        return TITLE_PANEL;
    return null;
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
