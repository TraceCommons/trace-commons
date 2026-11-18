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
    FOCUS_LEAVE, FOCUS_RESTORE, FOCUS_UNSET, FocusKeeper, MARGIN, MARKER_PANEL, MARKER_RAIL, ROLE_PANEL,
    ROLE_RAIL, couldBeCallers, focusAfter, frameFor, isMarked, markerFor, monitorFor,
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

test('each role has its own hidden marker, and nothing else has one', () => {
    assert.equal(markerFor(ROLE_RAIL), MARKER_RAIL);
    assert.equal(markerFor(ROLE_PANEL), MARKER_PANEL);
    assert.notEqual(MARKER_RAIL, MARKER_PANEL);
    assert.equal(markerFor('main'), null);
    assert.equal(markerFor(''), null);
});

// A window as Mutter describes it to the extension: all three of the app's
// windows share its pid, its bus name and its title (the brand name).
function metaWindow(pid, bus, path) {
    return {
        get_pid: () => pid,
        get_gtk_unique_bus_name: () => bus,
        get_gtk_window_object_path: () => path,
        get_title: () => 'Brand',
    };
}

test('only a window of the calling connection with an exported path is looked at', () => {
    const rail = metaWindow(7, ':1.5', '/app/window/2');
    assert.ok(couldBeCallers(rail, 7, ':1.5'));
    // Another process, another connection of the same process, or a window
    // GTK exported nothing for (a plain GtkWindow): never.
    assert.ok(!couldBeCallers(rail, 8, ':1.5'));
    assert.ok(!couldBeCallers(rail, 7, ':1.6'));
    assert.ok(!couldBeCallers(metaWindow(7, ':1.5', null), 7, ':1.5'));
    assert.ok(!couldBeCallers(metaWindow(7, null, null), 7, ':1.5'));
});

test('the main window is never placed: it carries no marker, whatever its title', () => {
    // The main window passes the first check (same pid, bus and title) ...
    const main = metaWindow(7, ':1.5', '/app/window/1');
    assert.ok(couldBeCallers(main, 7, ':1.5'));
    // ... and is refused for either role, by the actions it exports.
    for (const role of [ROLE_RAIL, ROLE_PANEL])
        assert.ok(!isMarked(role, ['about', 'show-help-overlay']), role);
    assert.ok(!isMarked(ROLE_RAIL, []));
    assert.ok(!isMarked(ROLE_RAIL, null));
    // Each marked window answers for its own role alone.
    assert.ok(isMarked(ROLE_RAIL, [MARKER_RAIL]));
    assert.ok(!isMarked(ROLE_PANEL, [MARKER_RAIL]));
    assert.ok(isMarked(ROLE_PANEL, [MARKER_PANEL]));
    assert.ok(!isMarked('main', [MARKER_RAIL, MARKER_PANEL]));
});

const MONITORS = [
    {x: 0, y: 0, width: 1920, height: 1080},
    {x: 1920, y: 0, width: 2560, height: 1440},
];

test('the panel opens on the monitor that was clicked', () => {
    assert.equal(monitorFor(MONITORS, [100, 10]), 0);
    assert.equal(monitorFor(MONITORS, [1920, 10]), 1);
    assert.equal(monitorFor(MONITORS, [4479, 1439]), 1);
    // Off every monitor: none, and the extension keeps the window's own.
    assert.equal(monitorFor(MONITORS, [4480, 10]), -1);
    assert.equal(monitorFor(MONITORS, [-1, -1]), -1);
});

// A window with signals, as a Meta.Window has: connect, disconnect, emit.
class FakeWindow {
    constructor(name, props = {}) {
        this.name = name;
        Object.assign(this, props);
        this._handlers = new Map();
        this._next = 1;
        this.focused = [];
    }

    connect(signal, handler) {
        const id = this._next++;
        this._handlers.set(id, {signal, handler});
        return id;
    }

    disconnect(id) {
        this._handlers.delete(id);
    }

    emit(signal) {
        for (const {signal: s, handler} of [...this._handlers.values()]) {
            if (s === signal)
                handler(this);
        }
    }

    connections() {
        return this._handlers.size;
    }

    focus(time) {
        this.focused.push(time);
    }
}

function fakeDisplay() {
    return {
        focus_window: null,
        roundtrips: 0,
        unsets: [],
        get_current_time_roundtrip() {
            this.roundtrips += 1;
            return 1000 + this.roundtrips;
        },
        unset_input_focus(time) {
            this.unsets.push(time);
        },
    };
}

// The keeper with a rail of the given openness, every window on the active
// workspace unless it says otherwise.
function keeper(display, rail) {
    return new FocusKeeper(display, {
        isRail: w => w === rail,
        isOpen: w => w === rail && Boolean(rail.open),
        onActiveWorkspace: w => w.workspace !== 'other',
        isMinimized: w => Boolean(w.minimized),
    });
}

test('focus comes back from the closed rail to the window noted before it', () => {
    const display = fakeDisplay();
    const rail = new FakeWindow('rail', {open: false});
    const editor = new FakeWindow('editor');
    const focus = keeper(display, rail);
    display.focus_window = editor;
    focus.onFocus();
    display.focus_window = rail;
    focus.onFocus();
    assert.deepEqual(editor.focused, [1001]);
    assert.deepEqual(display.unsets, []);
});

test('a noted window that starts to go is never given focus again', () => {
    const display = fakeDisplay();
    const rail = new FakeWindow('rail', {open: false});
    const editor = new FakeWindow('editor');
    const focus = keeper(display, rail);
    display.focus_window = editor;
    focus.onFocus();
    // Mutter sends unmanaging as a window starts to go; focus may land on
    // the rail before unmanaged.
    editor.emit('unmanaging');
    assert.ok(!focus.isManaged(editor));
    display.focus_window = rail;
    focus.onFocus();
    assert.deepEqual(editor.focused, [], 'focus given to a window going away');
    assert.equal(display.unsets.length, 1);
    assert.equal(editor.connections(), 0, 'the keeper still listens to a window that went');
});

test('forgetting lets go of the window it listened to', () => {
    const display = fakeDisplay();
    const rail = new FakeWindow('rail', {open: false});
    const editor = new FakeWindow('editor');
    const focus = keeper(display, rail);
    display.focus_window = editor;
    focus.onFocus();
    assert.equal(editor.connections(), 1);
    focus.forget();
    assert.equal(editor.connections(), 0);
    display.focus_window = rail;
    focus.onFocus();
    assert.deepEqual(editor.focused, []);
});

test('the clock is asked only when focus is moved', () => {
    const display = fakeDisplay();
    const rail = new FakeWindow('rail', {open: true});
    const editor = new FakeWindow('editor');
    const focus = keeper(display, rail);
    display.focus_window = editor;
    focus.onFocus();
    display.focus_window = rail;
    focus.onFocus();
    display.focus_window = null;
    focus.onFocus();
    assert.equal(display.roundtrips, 0, 'a round trip to the X server for nothing');
    rail.open = false;
    display.focus_window = rail;
    focus.onFocus();
    assert.equal(display.roundtrips, 1);
});
