// SPDX-License-Identifier: MIT OR Apache-2.0
//
// node --test crates/trace-commons-contributor-gtk/gnome-shell-extension/edge-surfaces@tracecommons.ai/tests/placement.test.js
//
// Node's own runner and assert: nothing is installed for this. The rail's
// numbers are the Mac's: EdgeRailGeometry.frame (macos/Sources/TCDesign/
// Components/EdgeRail.swift), run in a Swift probe on a 1920 x 1080 screen
// whose bar is 32 high, gave (1916, 436, 4 x 240) closed and
// (1436, 216, 484 x 680) open, in top-left coordinates. The panel's are
// geometry::panel_anchor's (src/edge/geometry.rs).

import {test} from 'node:test';
import assert from 'node:assert/strict';

import {
    FOCUS_LEAVE, FOCUS_RESTORE, FOCUS_UNSET, MARGIN, ROLE_PANEL, ROLE_RAIL, TITLE_PANEL, TITLE_RAIL,
    focusAfter, frameFor, titleFor,
} from '../placement.js';

const AREA = {x: 0, y: 32, width: 1920, height: 1048};

test('the closed rail is the Mac zone: right edge, centred', () => {
    assert.deepEqual(frameFor(ROLE_RAIL, AREA, {width: 4, height: 240}, [0, 0]), {x: 1916, y: 436});
});

test('the open rail is the Mac panel: same edge, still centred', () => {
    assert.deepEqual(frameFor(ROLE_RAIL, AREA, {width: 484, height: 680}, [0, 0]), {x: 1436, y: 216});
});

test('the rail follows a monitor away from the origin', () => {
    const right = {x: 1920, y: 0, width: 2560, height: 1440};
    assert.deepEqual(frameFor(ROLE_RAIL, right, {width: 484, height: 680}, [0, 0]), {x: 3996, y: 380});
});

test('an odd leftover leaves the half pixel below, in whole pixels', () => {
    assert.equal(frameFor(ROLE_RAIL, {x: 0, y: 0, width: 100, height: 241}, {width: 4, height: 240}, [0, 0]).y, 0);
});

test('the panel is centred on the press and kept inside the margin', () => {
    const size = {width: 285, height: 400};
    assert.equal(MARGIN, 8);
    assert.deepEqual(frameFor(ROLE_PANEL, AREA, size, [960, 10]), {x: 818, y: 40});
    assert.equal(frameFor(ROLE_PANEL, AREA, size, [1915, 10]).x, 1920 - 285 - 8);
    assert.equal(frameFor(ROLE_PANEL, AREA, size, [3, 10]).x, 8);
});

test('a press in the lower half puts the panel against the bottom margin', () => {
    const size = {width: 285, height: 400};
    assert.equal(frameFor(ROLE_PANEL, AREA, size, [960, 1070]).y, 32 + 1048 - 400 - 8);
    assert.equal(frameFor(ROLE_PANEL, AREA, size, [960, 555]).y, 40);
    assert.equal(frameFor(ROLE_PANEL, AREA, size, [960, 556]).y, 672);
});

// A model of the shell for the focus rule: each window says whether it is
// the rail and open, still managed, on the active workspace, minimised.
function shell(...windows) {
    return {
        isRail: w => Boolean(w.rail),
        isOpen: w => Boolean(w.open),
        isManaged: w => w.managed !== false,
        onActiveWorkspace: w => w.workspace === 'active',
        isMinimized: w => Boolean(w.minimized),
        windows,
    };
}

test('a closed rail never keeps focus: it goes back to the window before it', () => {
    const rail = {rail: true, open: false, workspace: 'active'};
    const editor = {workspace: 'active'};
    assert.deepEqual(focusAfter(rail, editor, shell(rail, editor)), {action: FOCUS_RESTORE, window: editor});
});

test('an open rail keeps the keyboard, so Escape and Tab reach it', () => {
    const rail = {rail: true, open: true, workspace: 'active'};
    const editor = {workspace: 'active'};
    assert.deepEqual(focusAfter(rail, editor, shell(rail, editor)), {action: FOCUS_LEAVE, window: null});
});

test('focus is never given back across workspaces', () => {
    const rail = {rail: true, open: false, workspace: 'active'};
    const elsewhere = {workspace: 'other'};
    assert.deepEqual(focusAfter(rail, elsewhere, shell(rail, elsewhere)), {action: FOCUS_UNSET, window: null});
});

test('a minimised window is never brought back to take focus', () => {
    const rail = {rail: true, open: false, workspace: 'active'};
    const minimised = {workspace: 'active', minimized: true};
    assert.deepEqual(focusAfter(rail, minimised, shell(rail, minimised)), {action: FOCUS_UNSET, window: null});
});

test('a window that is going away is never given focus', () => {
    const rail = {rail: true, open: false, workspace: 'active'};
    const closing = {workspace: 'active', managed: false};
    assert.deepEqual(focusAfter(rail, closing, shell(rail, closing)), {action: FOCUS_UNSET, window: null});
});

test('with no window to go back to, focus is unset rather than left on the rail', () => {
    const rail = {rail: true, open: false, workspace: 'active'};
    const other = {rail: true, open: false, workspace: 'active'};
    assert.deepEqual(focusAfter(rail, null, shell(rail)), {action: FOCUS_UNSET, window: null});
    assert.deepEqual(focusAfter(rail, rail, shell(rail)), {action: FOCUS_UNSET, window: null});
    assert.deepEqual(focusAfter(rail, other, shell(rail, other)), {action: FOCUS_UNSET, window: null});
});

test('focus on any other window is left alone', () => {
    const rail = {rail: true, open: false, workspace: 'active'};
    const editor = {workspace: 'active'};
    assert.deepEqual(focusAfter(editor, rail, shell(rail, editor)), {action: FOCUS_LEAVE, window: null});
    assert.deepEqual(focusAfter(null, editor, shell(editor)), {action: FOCUS_LEAVE, window: null});
});

test('each role places only the window of its own title', () => {
    assert.equal(titleFor(ROLE_RAIL), TITLE_RAIL);
    assert.equal(titleFor(ROLE_PANEL), TITLE_PANEL);
    assert.notEqual(TITLE_RAIL, TITLE_PANEL);
    assert.equal(titleFor('main'), null);
});
