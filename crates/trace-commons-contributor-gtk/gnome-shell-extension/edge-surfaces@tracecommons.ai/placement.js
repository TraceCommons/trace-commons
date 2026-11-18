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
