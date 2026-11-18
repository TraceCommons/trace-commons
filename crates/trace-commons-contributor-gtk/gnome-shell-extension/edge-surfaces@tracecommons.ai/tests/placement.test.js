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

import {MARGIN, ROLE_PANEL, ROLE_RAIL, focusToRestore, frameFor} from '../placement.js';

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

test('a rail that takes focus gives it back to the window before it', () => {
    const rail = {};
    const editor = {};
    const isRail = w => w === rail;
    assert.equal(focusToRestore(rail, editor, isRail), editor);
    assert.equal(focusToRestore(editor, rail, isRail), null);
    assert.equal(focusToRestore(rail, null, isRail), null);
    assert.equal(focusToRestore(rail, rail, isRail), null);
    assert.equal(focusToRestore(null, editor, isRail), null);
});
