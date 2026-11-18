// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Where the extension puts each window. No GNOME imports, so it runs
// under plain node as well (the crate's tests read its numbers; see
// src/edge/gnome.rs).

export const ROLE_RAIL = 'rail';
export const ROLE_PANEL = 'panel';
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

/**
 * The rail never keeps keyboard focus: on the Mac it is a non-activating
 * panel that "never takes focus from the app in front". Mutter focuses a
 * window when it is clicked, so when focus lands on a rail this names the
 * window to give it back to, or null to leave it where it is.
 */
export function focusToRestore(focused, previous, isRail) {
    if (!focused || !isRail(focused))
        return null;
    if (!previous || previous === focused || isRail(previous))
        return null;
    return previous;
}
