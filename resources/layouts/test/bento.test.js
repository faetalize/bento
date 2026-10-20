// Tests for bento's grid layout, `bento.js`.
//
// The first part ports the behaviour tests from bento's `src/layout/tests/grid.rs`. The output is
// 1280 × 720 with 16 px gaps, which makes the grid 22 columns by 12 rows of roughly 41 × 43 px
// cells. Like the Rust tests, windows open 100 px wide, so 2 cells wide: the Rust test windows
// pick that width themselves, here it comes from `default-column-width`. Positions are on screen,
// relative to the view, rounded like bento renders them, so the expectations are the Rust
// snapshots.
//
// The second part ports the unit tests of the grid arithmetic from `src/layout/grid.rs`.

import { describe, test } from "node:test";
import assert from "node:assert/strict";

import { TestHost } from "./host.js";
import {
    Area,
    CellMetrics,
    DEFAULT_ROWS,
    Direction,
    Grid,
    cellsForDelta,
    nextPresetSpan,
    scaleSpan,
} from "../bento.js";

const { Left, Right, Up, Down } = Direction;

/** Distance between the left edges of neighboring grid columns in the test output. */
const COL_STEP = (1280 - 16 * 23) / 22 + 16;
/** Distance between the top edges of neighboring grid rows in the test output. */
const ROW_STEP = (720 - 16 * 13) / 12 + 16;

async function setup(opts = {}) {
    return TestHost.load("../bento.js", {
        ...opts,
        config: { defaultColumnWidth: { fixed: 100 }, ...(opts.config ?? {}) },
    });
}

/** Rounds halves away from zero, like bento rounds render positions to pixels. */
function px(value) {
    return Math.sign(value) * Math.round(Math.abs(value)) + 0;
}

/** Tiles in the order they opened, on screen, like `format_tiles` in the Rust tests. */
function tiles(host, ids = [...host.windows.keys()]) {
    return ids
        .sort((a, b) => a - b)
        .map((id) => {
            const r = host.screenRect(id);
            return `${px(r.w)} × ${px(r.h)} at x:${px(r.x)} y:${px(r.y)}`;
        });
}

const set = (value) => ({ op: "set", value, unit: "%" });
const adjust = (value) => ({ op: "adjust", value, unit: "%" });
/** A proportion that gives exactly `n` rows. */
const rows = (n) => set((n * 100) / 12 + 0.1);

function halfHeight(host, id) {
    host.actionOn(id, "set-window-height", set(50));
}

function repeat(n, fn) {
    for (let i = 0; i < n; i++) fn();
}

function edges(given) {
    return { left: false, right: false, top: false, bottom: false, ...given };
}

function resizeBegin(host, id, given) {
    return host.send({
        type: "resize-begin",
        id,
        resize: { edges: edges(given), delta: { x: 0, y: 0 } },
    }).result;
}

function resizeUpdate(host, id, given, dx, dy) {
    host.send({
        type: "resize-update",
        id,
        resize: { edges: edges(given), delta: { x: dx, y: dy } },
    });
}

function resizeEnd(host, id) {
    host.send({ type: "resize-end", id });
}

/** Drags a window grabbed at `start` on screen and drops it with the pointer at `end`. */
function drag(host, id, start, end) {
    const camera = host.camera;
    const r = host.rect(id);
    const pointer = { x: end.x + camera.x, y: end.y + camera.y };
    const rect = { x: r.x + end.x - start.x, y: r.y + end.y - start.y, w: r.w, h: r.h };
    host.drag(id, pointer, rect);
}

/** Lifts a window out of the layout for a drag, without dropping it. */
function lift(host, id) {
    host.send({ type: "remove", id, reason: "drag" });
}

/** Windows 1 full height, window 2 half height on top to its right, with a hole below 2. */
async function setUpHole() {
    const host = await setup();
    host.open();
    host.open();
    halfHeight(host, 2);
    return host;
}

/** Windows 1 and 2 on the top half, window 3 in the bottom half under window 1. */
async function setUpTwoOverOne() {
    const host = await setup();
    for (const id of [1, 2, 3]) {
        host.open();
        halfHeight(host, id);
    }
    repeat(6, () => host.action("move-window-down"));
    return host;
}

/** Window 1 full height on the left, window 2 on the top half to its right, 3 under 2. */
async function setUpOneAndStack() {
    const host = await setup();
    host.open();
    host.open();
    halfHeight(host, 2);
    host.open();
    halfHeight(host, 3);
    repeat(6, () => host.action("move-window-down"));
    return host;
}

/** Two windows of `width` percent stacked on the left, then a full-height one, focused. */
async function setUpStackAndOne(width, opts = {}) {
    const host = await setup(opts);
    for (const id of [1, 2, 3]) {
        host.open();
        host.actionOn(id, "set-window-width", set(width));
    }
    halfHeight(host, 1);
    halfHeight(host, 2);
    host.activate(2);
    repeat(6, () => host.action("move-window-down"));
    host.action("move-column-left");
    host.activate(3);
    return host;
}

/** Windows 1 and 2 stacked on the left, window 3 on the right. */
async function setUpStackOfTwo() {
    const host = await setup();
    host.open();
    host.open();
    host.open();
    host.activate(2);
    host.action("consume-or-expel-window-left");
    return host;
}

describe("grid layout", () => {
    test("new windows open to the right", async () => {
        const host = await setup();
        host.open();
        host.open();
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 688 at x:131 y:16"]);
        assert.equal(host.focused, 2);
        assert.deepEqual(host.errors, []);
    });

    test("shrinking height leaves a hole", async () => {
        const host = await setUpHole();
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 336 at x:131 y:16"]);
    });

    test("moving down into a hole goes one cell at a time", async () => {
        const host = await setUpHole();
        host.action("move-window-down");
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 336 at x:131 y:75"]);

        // Moving past the bottom does nothing.
        repeat(10, () => host.action("move-window-down"));
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 336 at x:131 y:368"]);
        assert.equal(host.action("move-window-down"), false);
    });

    test("moving into a window swaps", async () => {
        const host = await setup();
        host.open();
        host.open();
        assert.equal(host.action("move-column-left"), true);

        // Window 2 is now on the left.
        assert.deepEqual(tiles(host), ["99 × 688 at x:131 y:16", "99 × 688 at x:16 y:16"]);
        assert.equal(host.focused, 2);

        // Column 0 is the left edge of the grid with gravity.
        assert.equal(host.action("move-column-left"), false);
    });

    test("growing down pushes the window below", async () => {
        const host = await setUpHole();

        // Put window 3 below window 2 by opening it and moving it down. Left gravity slides it
        // under window 2.
        host.open();
        halfHeight(host, 3);
        repeat(6, () => host.action("move-window-down"));
        assert.deepEqual(tiles(host), [
            "99 × 688 at x:16 y:16",
            "99 × 336 at x:131 y:16",
            "99 × 336 at x:131 y:368",
        ]);

        // Focus moves geometrically.
        host.action("focus-window-up");
        assert.equal(host.focused, 2);
        host.action("focus-window-down");
        assert.equal(host.focused, 3);
        host.action("focus-column-left");
        assert.equal(host.focused, 1);

        // Growing 2 down pushes 3, which doesn't fit below anymore and goes to the right.
        host.actionOn(2, "set-window-height", set(75));
        assert.deepEqual(tiles(host), [
            "99 × 688 at x:16 y:16",
            "99 × 512 at x:131 y:16",
            "99 × 336 at x:246 y:368",
        ]);
    });

    test("closing a window collapses empty columns", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.open();
        host.close(2);

        // Window 3 slides into the place of window 2, and the view stays still.
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 688 at x:131 y:16"]);
    });

    test("switching preset width snaps to cells", async () => {
        const host = await setup();
        host.open();

        // From 2 cells to the next larger preset, 1/3 (7 cells).
        host.action("switch-preset-column-width");
        assert.deepEqual(tiles(host), ["386 × 688 at x:16 y:16"]);

        // Then 1/2 (11 cells).
        host.action("switch-preset-column-width");
        assert.deepEqual(tiles(host), ["616 × 688 at x:16 y:16"]);

        // And back.
        host.action("switch-preset-column-width-back");
        assert.deepEqual(tiles(host), ["386 × 688 at x:16 y:16"]);
    });

    test("dragging the right edge snaps and pushes", async () => {
        const host = await setup();
        host.open();
        host.open();
        assert.equal(resizeBegin(host, 1, { right: true }), true);

        // A bit over 2.5 cells snaps to 3 more cells.
        resizeUpdate(host, 1, { right: true }, COL_STEP * 2.6, 0);

        // Window 1 is 5 cells wide and pushed window 2 along.
        assert.deepEqual(tiles(host), ["271 × 688 at x:16 y:16", "99 × 688 at x:303 y:16"]);

        // Dragging back puts window 2 back where it was.
        resizeUpdate(host, 1, { right: true }, COL_STEP * 0.4, 0);
        resizeEnd(host, 1);
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 688 at x:131 y:16"]);
    });

    test("dragging the bottom edge leaves a hole", async () => {
        const host = await setup();
        host.open();
        host.resize(1, { bottom: true }, { x: 0, y: -ROW_STEP * 4 });

        // 8 rows tall.
        assert.deepEqual(tiles(host), ["99 × 453 at x:16 y:16"]);
    });

    test("dragging the left edge stops at the start of the grid", async () => {
        const host = await setup();
        host.open();
        host.open();
        resizeBegin(host, 2, { left: true });
        resizeUpdate(host, 2, { left: true }, -COL_STEP * 3, 0);

        // Window 1 is already at the start of the grid, so window 2 can't push it any further,
        // and doesn't grow.
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 688 at x:131 y:16"]);

        // Shrinking from the left still works, keeping the right edge still.
        resizeUpdate(host, 2, { left: true }, COL_STEP, 0);
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "42 × 688 at x:188 y:16"]);

        // Letting go drops it next to window 1.
        resizeEnd(host, 2);
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "41 × 688 at x:131 y:16"]);
    });

    test("dropping a window into a cell", async () => {
        // Window 1 full height, window 2 on top to its right, window 3 half height further right.
        const host = await setup();
        host.open();
        host.open();
        halfHeight(host, 2);
        host.open();
        halfHeight(host, 3);
        assert.deepEqual(tiles(host), [
            "99 × 688 at x:16 y:16",
            "99 × 336 at x:131 y:16",
            "99 × 336 at x:246 y:16",
        ]);

        // Grab window 3 near its top-left corner and drop it into the hole under window 2.
        const start = { x: 16 + 4 * COL_STEP + 4, y: 16 + 4 };
        const end = { x: 16 + 2 * COL_STEP + 4, y: 16 + 6 * ROW_STEP + 4 };

        // The highlight shows where it lands.
        const r = host.rect(3);
        const pointer = { x: end.x + host.camera.x, y: end.y };
        const rect = { x: r.x + end.x - start.x, y: r.y + end.y - start.y, w: r.w, h: r.h };
        assert.deepEqual(host.dropTarget(3, pointer, rect), { x: 115, y: 368, w: 99, h: 336 });

        drag(host, 3, start, end);
        assert.deepEqual(tiles(host), [
            "99 × 688 at x:16 y:16",
            "99 × 336 at x:131 y:16",
            "99 × 336 at x:131 y:368",
        ]);
        assert.equal(host.focused, 3);
    });

    test("dropping on the edge of a window splits it", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.open();

        // Drop window 3 at the top edge of window 1: 3 takes the top half and 1 keeps the bottom.
        drag(host, 3, { x: 16 + 4 * COL_STEP + 50, y: 300 }, { x: 16 + 50, y: 16 + 10 });
        assert.deepEqual(tiles(host), [
            "99 × 336 at x:16 y:368",
            "99 × 688 at x:131 y:16",
            "99 × 336 at x:16 y:16",
        ]);

        // The highlight for dropping window 2 at the left edge of window 1 is the left column.
        const pointer = { x: 16 + 5 + host.camera.x, y: 16 + 9 * ROW_STEP };
        assert.deepEqual(host.dropTarget(2, pointer, host.rect(2)), {
            x: 0,
            y: 368,
            w: 41,
            h: 336,
        });

        // Drop window 2 at the left edge of window 1: 2 takes the left column, 1 the right one.
        drag(host, 2, { x: 16 + 2 * COL_STEP + 50, y: 300 }, { x: 16 + 5, y: 16 + 9 * ROW_STEP });
        assert.deepEqual(tiles(host), [
            "42 × 336 at x:73 y:368",
            "41 × 336 at x:16 y:368",
            "99 × 336 at x:16 y:16",
        ]);
    });

    test("dragging the bottom window of a stack keeps the view still", async () => {
        // Window 2 under window 1, focused.
        const host = await setup();
        host.open();
        halfHeight(host, 1);
        host.open();
        halfHeight(host, 2);
        repeat(6, () => host.action("move-window-down"));
        host.action("move-column-left");
        assert.equal(host.focused, 2);
        assert.deepEqual(tiles(host), ["99 × 336 at x:16 y:16", "99 × 336 at x:16 y:368"]);

        // Lift window 2 out of the layout. Window 1 hasn't jumped.
        lift(host, 2);
        assert.deepEqual(tiles(host, [1]), ["99 × 336 at x:16 y:16"]);
        assert.equal(host.focused, 1);
    });

    test("dragging a window that fell left keeps the view still", async () => {
        // Windows 1 and 2 on the top half, then window 3 opens right of window 2, and falls under
        // window 1 when moved to the bottom half.
        const host = await setUpTwoOverOne();
        assert.equal(host.focused, 3);
        assert.deepEqual(tiles(host), [
            "99 × 336 at x:16 y:16",
            "99 × 336 at x:131 y:16",
            "99 × 336 at x:16 y:368",
        ]);

        // Windows 1 and 2 stay where they were.
        lift(host, 3);
        assert.deepEqual(tiles(host, [1, 2]), ["99 × 336 at x:16 y:16", "99 × 336 at x:131 y:16"]);
    });

    test("closing a window keeps focus on another", async () => {
        const host = await setUpTwoOverOne();
        host.activate(2);
        assert.deepEqual(tiles(host), [
            "99 × 336 at x:16 y:16",
            "99 × 336 at x:131 y:16",
            "99 × 336 at x:16 y:368",
        ]);

        // Closing window 1 lets window 2 slide left, which reorders the windows.
        host.close(1);
        assert.equal(host.focused, 2);
        assert.deepEqual(tiles(host), ["99 × 336 at x:16 y:16", "99 × 336 at x:16 y:368"]);
    });

    test("closing the focused window focuses the one above", async () => {
        const host = await setUpTwoOverOne();
        // Focus away and back so that closing window 3 doesn't go back to window 2 it was opened
        // from.
        host.activate(1);
        host.activate(3);
        host.close(3);
        assert.equal(host.focused, 1);
    });

    test("closing a new window goes back to the previous one", async () => {
        // Window 1 on top of window 2, both on the left.
        const host = await setup();
        host.open();
        halfHeight(host, 1);
        host.open();
        halfHeight(host, 2);
        repeat(6, () => host.action("move-window-down"));
        host.action("move-column-left");
        host.activate(1);
        assert.deepEqual(tiles(host), ["99 × 336 at x:16 y:16", "99 × 336 at x:16 y:368"]);

        // Window 3 opens right of window 1, after window 2 in the order.
        host.open();
        assert.equal(host.focused, 3);

        host.close(3);
        assert.equal(host.focused, 1);
    });

    test("swiping snaps to window edges with stacked windows", async () => {
        // Windows 1 and 2 stacked on the left, then windows 3 and 4, all half the view wide.
        const host = await setup();
        for (const id of [1, 2, 3, 4]) {
            host.open();
            host.actionOn(id, "set-window-width", set(50));
        }
        halfHeight(host, 1);
        halfHeight(host, 2);
        host.activate(2);
        repeat(6, () => host.action("move-window-down"));
        host.action("move-column-left");
        host.activate(1);
        assert.deepEqual(tiles(host), [
            "616 × 336 at x:16 y:16",
            "616 × 336 at x:16 y:368",
            "616 × 688 at x:648 y:16",
            "616 × 688 at x:1280 y:16",
        ]);

        // Swipe right by a bit over one window.
        host.swipe(700);

        // The view snaps to window 3 on the left and window 4 on the right, focusing window 4 as
        // the furthest in the direction of the swipe.
        assert.equal(host.focused, 4);
        assert.deepEqual(tiles(host), [
            "616 × 336 at x:-616 y:16",
            "616 × 336 at x:-616 y:368",
            "616 × 688 at x:16 y:16",
            "616 × 688 at x:648 y:16",
        ]);
    });

    test("swiping snaps where the swipe would coast to", async () => {
        const host = await setup();
        for (const id of [1, 2, 3, 4]) {
            host.open();
            host.actionOn(id, "set-window-width", set(50));
        }
        host.activate(1);

        // A small swipe that would coast far snaps to the far end.
        host.swipe(100, { x: 3000, y: 0 }, { x: host.camera.x + 2000, y: 0 });
        assert.equal(host.focused, 4);
        assert.deepEqual(tiles(host, [3, 4]), [
            "616 × 688 at x:16 y:16",
            "616 × 688 at x:648 y:16",
        ]);
    });

    test("stacked windows count as a single column for centering", async () => {
        const host = await setup({ config: { alwaysCenterSingleColumn: true } });
        host.open();
        halfHeight(host, 1);
        host.open();
        halfHeight(host, 2);
        repeat(6, () => host.action("move-window-down"));
        host.action("move-column-left");
        assert.deepEqual(tiles(host), ["99 × 336 at x:591 y:16", "99 × 336 at x:591 y:368"]);
    });

    test("centering visible columns uses their span on the grid", async () => {
        const host = await setUpStackAndOne(20);
        host.action("center-visible-columns");
        assert.deepEqual(tiles(host), [
            "214 × 336 at x:418 y:16",
            "214 × 336 at x:418 y:368",
            "214 × 688 at x:648 y:16",
        ]);
    });

    test("center on overflow compares with the window beside", async () => {
        const host = await setUpStackAndOne(60, { config: { centerFocusedColumn: "on-overflow" } });
        host.action("focus-column-left");
        assert.equal(host.focused, 1);
        // Window 1 and window 3 beside it don't fit together, so window 1 is centered. Window 2
        // under it is in the same column of cells and doesn't count.
        assert.deepEqual(tiles(host), [
            "731 × 336 at x:275 y:16",
            "731 × 336 at x:275 y:368",
            "731 × 688 at x:1022 y:16",
        ]);
    });

    test("moving a resized window to another workspace keeps its size", async () => {
        const from = await setup();
        from.open();
        from.actionOn(1, "set-window-width", set(33));
        from.actionOn(1, "set-window-height", set(50));

        const transfer = from.transferOut(1);
        assert.deepEqual(transfer.windows, [1]);
        from.send({ type: "remove", id: 1, reason: "move" });

        // The window keeps its id on the other workspace.
        const to = await setup();
        const win = to.makeWindow({});
        assert.equal(win.id, 1);
        to.send({
            type: "transfer-in",
            windows: [to.info(win)],
            data: transfer.data,
            activate: true,
        });
        assert.deepEqual(tiles(to), ["386 × 336 at x:16 y:16"]);
        assert.equal(to.focused, 1);
    });

    test("toggling full width restores the previous width", async () => {
        const host = await setup();
        host.open();
        host.actionOn(1, "set-window-width", set(33));
        host.action("maximize-column");
        assert.deepEqual(tiles(host), ["1248 × 688 at x:16 y:16"]);

        host.action("maximize-column");
        assert.deepEqual(tiles(host), ["386 × 688 at x:16 y:16"]);
    });

    test("adjusting width goes by the requested amount", async () => {
        // 10% of 22 columns is 2 columns.
        const host = await setup();
        host.open();
        host.actionOn(1, "set-window-width", adjust(10));
        assert.deepEqual(tiles(host), ["214 × 688 at x:16 y:16"]);

        // A small adjustment still changes the size by a cell.
        host.action("set-column-width", { op: "adjust", value: -10, unit: "px" });
        assert.deepEqual(tiles(host), ["156 × 688 at x:16 y:16"]);
    });

    test("floating and tiling again keeps the size and place", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.actionOn(2, "set-window-width", set(33));
        host.actionOn(2, "set-window-height", set(50));
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "386 × 336 at x:131 y:16"]);

        // The window floats where it was tiled, then is tiled again from there.
        const rect = host.rect(2);
        host.send({ type: "remove", id: 2, reason: "float" });
        host.send({
            type: "add",
            window: host.info(host.windows.get(2)),
            how: { reason: "tile", activate: true, rect, pointer: null, nextTo: null },
        });
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "386 × 336 at x:131 y:16"]);
    });

    test("default window height opens next to the active window", async () => {
        // Window 1 in the bottom half.
        const host = await setup();
        host.open();
        halfHeight(host, 1);
        repeat(6, () => host.action("move-window-down"));

        // Window 2 opens half height, lined up with window 1.
        host.open({ defaultHeight: { proportion: 0.5 } });
        assert.deepEqual(tiles(host), ["99 × 336 at x:16 y:368", "99 × 336 at x:131 y:368"]);
    });

    test("focusing the last column stays in the row", async () => {
        const host = await setUpOneAndStack();
        host.activate(1);
        host.action("focus-column-last");
        assert.deepEqual(tiles(host), [
            "99 × 688 at x:16 y:16",
            "99 × 336 at x:131 y:16",
            "99 × 336 at x:131 y:368",
        ]);

        // Window 3 comes last in the order, but window 2 is the last one in the top row.
        assert.equal(host.focused, 2);
    });

    test("focusing the top window goes up the stack", async () => {
        const host = await setUpOneAndStack();
        host.activate(3);
        host.action("focus-window-top");
        assert.equal(host.focused, 2);
    });

    test("focusing down or to the workspace below skips windows not below", async () => {
        // Window 3 is under window 1, not under window 2.
        const host = await setUpTwoOverOne();
        host.activate(2);

        // Nothing to do here, so bento switches to the workspace below.
        assert.equal(host.action("focus-window-or-workspace-down"), false);
        assert.equal(host.focused, 2);

        // Plain focus-window-down reaches it diagonally.
        assert.equal(host.action("focus-window-down"), true);
        assert.equal(host.focused, 3);
    });

    test("ipc positions follow grid columns", async () => {
        const host = await setUpTwoOverOne();

        // Windows 1 and 3 are stacked in the first column, window 2 is in the second.
        assert.deepEqual(
            [...host.ipc].sort((a, b) => a[0] - b[0]),
            [
                [1, [1, 1]],
                [2, [2, 1]],
                [3, [1, 2]],
            ],
        );
    });

    test("fullscreen pushes windows aside until it ends", async () => {
        // Window 2 half height, with window 3 under it.
        const host = await setup();
        host.open();
        host.open();
        halfHeight(host, 2);
        host.open();
        halfHeight(host, 3);
        repeat(6, () => host.action("move-window-down"));
        host.activate(2);
        host.requestMode(2, "fullscreen");

        // Window 2 covers the view, and window 3 is pushed past it rather than drawn under it.
        assert.deepEqual(tiles(host), [
            "99 × 688 at x:-115 y:16",
            "1280 × 720 at x:0 y:0",
            "99 × 336 at x:1321 y:368",
        ]);
        assert.equal(host.mode(2), "fullscreen");

        // Everything goes back once it ends.
        host.requestMode(2, "normal");
        assert.deepEqual(tiles(host), [
            "99 × 688 at x:16 y:16",
            "99 × 336 at x:131 y:16",
            "99 × 336 at x:131 y:368",
        ]);
        assert.equal(host.mode(2), "normal");
    });

    test("expanding fills the free cells in view", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.action("expand-column-to-available-width");

        // Window 2 grows up to the right edge of the view, and window 1 stays as is.
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "1133 × 688 at x:131 y:16"]);
        assert.equal(host.action("expand-column-to-available-width"), false);
    });

    test("consuming splits rows and expelling gives them back", async () => {
        const host = await setUpStackOfTwo();

        // Window 2 goes under window 1, which keeps the top half, and window 3 slides left.
        assert.equal(host.focused, 2);
        assert.deepEqual(tiles(host), [
            "99 × 336 at x:16 y:16",
            "99 × 336 at x:16 y:368",
            "99 × 688 at x:131 y:16",
        ]);

        // Window 2 is stacked now, so this expels it back out to the right.
        host.action("consume-or-expel-window-right");
        assert.equal(host.focused, 2);
        assert.deepEqual(tiles(host), [
            "99 × 688 at x:16 y:16",
            "99 × 688 at x:131 y:16",
            "99 × 688 at x:246 y:16",
        ]);
    });

    test("consuming into column brings the window on the right under", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.activate(1);
        host.action("consume-window-into-column");
        assert.equal(host.focused, 1);
        assert.deepEqual(tiles(host), ["99 × 336 at x:16 y:16", "99 × 336 at x:16 y:368"]);

        // Expelling the focused window gives window 2 its rows back.
        host.action("expel-window-from-column");
        assert.equal(host.focused, 1);
        assert.deepEqual(tiles(host), ["99 × 688 at x:131 y:16", "99 × 688 at x:16 y:16"]);
    });

    test("tabbing gathers the stack and untabbing splits it", async () => {
        const host = await setUpStackOfTwo();
        host.activate(1);
        host.action("toggle-column-tabbed-display");

        // Windows 1 and 2 are tabs in the area they shared, with window 1 in front.
        assert.equal(host.focused, 1);
        assert.deepEqual(tiles(host), [
            "99 × 688 at x:16 y:16",
            "99 × 688 at x:16 y:16",
            "99 × 688 at x:131 y:16",
        ]);
        assert.equal(host.isVisible(1), true);
        assert.equal(host.isVisible(2), false);

        // Focusing down goes to the next tab.
        host.action("focus-window-down");
        assert.equal(host.focused, 2);
        assert.equal(host.isVisible(1), false);
        assert.equal(host.isVisible(2), true);

        host.action("toggle-column-tabbed-display");
        assert.equal(host.focused, 2);
        assert.deepEqual(tiles(host), [
            "99 × 336 at x:16 y:16",
            "99 × 336 at x:16 y:368",
            "99 × 688 at x:131 y:16",
        ]);
        assert.equal(host.isVisible(1), true);
        assert.equal(host.tabs.size, 0);
    });

    test("consuming into tabs adds a tab and expelling takes it out", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.activate(1);
        host.action("toggle-column-tabbed-display");
        host.activate(2);
        host.action("consume-or-expel-window-left");

        // Window 2 is a second tab in window 1's area.
        assert.equal(host.focused, 2);
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 688 at x:16 y:16"]);
        assert.deepEqual(
            [...host.ipc].sort((a, b) => a[0] - b[0]),
            [
                [1, [1, 1]],
                [2, [1, 2]],
            ],
        );

        host.action("consume-or-expel-window-right");
        assert.equal(host.focused, 2);
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 688 at x:131 y:16"]);
    });

    test("swapping past a stack keeps the view still", async () => {
        // Window 1 three rows tall, windows 2 and 3 stacked to its right within its rows, and
        // window 4 under all of them, spanning both.
        const host = await setup();
        host.open();
        host.actionOn(1, "set-window-height", rows(3));
        host.open();
        host.actionOn(2, "set-window-height", rows(2));
        host.open();
        host.actionOn(3, "set-window-height", rows(1));
        repeat(2, () => host.action("move-window-down"));
        host.open();
        host.actionOn(4, "set-window-width", set(18.2));
        host.actionOn(4, "set-window-height", rows(2));
        repeat(3, () => host.action("move-window-down"));
        assert.deepEqual(tiles(host), [
            "99 × 160 at x:16 y:16",
            "99 × 101 at x:131 y:16",
            "99 × 43 at x:131 y:133",
            "214 × 101 at x:16 y:192",
        ]);

        // Window 1 trades places with the stack of windows 2 and 3. Window 4 isn't involved, so
        // it stays where it is on screen, and so does the view.
        host.activate(1);
        host.action("move-column-right");
        assert.deepEqual(tiles(host), [
            "99 × 160 at x:131 y:16",
            "99 × 101 at x:16 y:16",
            "99 × 43 at x:16 y:133",
            "214 × 101 at x:16 y:192",
        ]);
    });

    test("dragging the left edge without gravity pushes past column 0", async () => {
        const host = await setup({ options: { gravity: false } });
        host.open();
        host.open();
        resizeBegin(host, 2, { left: true });
        resizeUpdate(host, 2, { left: true }, -COL_STEP * 3, 0);

        // Without gravity there's no left edge to stop at: window 2 grows 3 cells to the left,
        // pushing window 1 past column 0, and its right edge stays at x = 230.
        assert.deepEqual(tiles(host), ["99 × 688 at x:-156 y:16", "271 × 688 at x:-41 y:16"]);
    });
});

describe("split and swap", () => {
    /** A and B stacked on the left half, C on the right half at full height. */
    async function setUpStackAndOne() {
        const host = await setup({ config: { defaultColumnWidth: { proportion: 0.5 } } });
        const a = host.open();
        const b = host.open();
        host.action("consume-or-expel-window-left");
        const c = host.open();
        assert.deepEqual(tiles(host), [
            "616 × 336 at x:16 y:16",
            "616 × 336 at x:16 y:368",
            "616 × 688 at x:648 y:16",
        ]);
        return { host, a, b, c };
    }

    test("splitting side by side gives the window the half facing it", async () => {
        const { host, a, c } = await setUpStackAndOne();

        // The first press picks A and highlights it.
        host.activate(a);
        assert.equal(host.action("split-h"), true);
        assert.deepEqual(host.hint, { x: 0, y: 16, w: 616, h: 336 });

        // The second press moves C into A's right half; A keeps the larger half.
        host.activate(c);
        assert.equal(host.action("split-h"), true);
        assert.equal(host.hint, null);
        assert.deepEqual(tiles(host), [
            "329 × 336 at x:16 y:16",
            "616 × 336 at x:16 y:368",
            "271 × 336 at x:361 y:16",
        ]);
        assert.equal(host.focused, c);
    });

    test("splitting the bottom window of a stack", async () => {
        const { host, b, c } = await setUpStackAndOne();
        host.activate(b);
        host.action("split-h");
        host.activate(c);
        host.action("split-h");
        assert.deepEqual(tiles(host), [
            "616 × 336 at x:16 y:16",
            "329 × 336 at x:16 y:368",
            "271 × 336 at x:361 y:368",
        ]);
    });

    test("splitting stacked gives the window the half facing it", async () => {
        const host = await setup({ config: { defaultColumnWidth: { proportion: 0.5 } } });
        const a = host.open();
        const b = host.open();
        host.action("split-v");
        host.activate(a);
        host.action("split-v");

        // A and B are level, so A goes below B, and the column it left collapses.
        assert.deepEqual(tiles(host), ["616 × 336 at x:16 y:368", "616 × 336 at x:16 y:16"]);
        assert.equal(host.focused, a);
        assert.equal(host.action("focus-window-up"), true);
        assert.equal(host.focused, b);
    });

    test("swapping trades places and sizes", async () => {
        const { host, a, c } = await setUpStackAndOne();
        host.activate(a);
        host.action("swap");
        host.activate(c);
        host.action("swap");
        assert.deepEqual(tiles(host), [
            "616 × 688 at x:648 y:16",
            "616 × 336 at x:16 y:368",
            "616 × 336 at x:16 y:16",
        ]);
    });

    test("picking the same window again cancels", async () => {
        const { host, a } = await setUpStackAndOne();
        host.activate(a);
        const before = tiles(host);
        host.action("swap");
        host.action("swap");
        assert.equal(host.hint, null);
        assert.deepEqual(tiles(host), before);
    });

    test("a picked window that closes ends the action", async () => {
        const { host, a, c } = await setUpStackAndOne();
        host.activate(a);
        host.action("split-h");
        host.close(a);
        assert.equal(host.hint, null);

        // The next press starts over.
        host.activate(c);
        host.action("split-h");
        assert.deepEqual(host.hint, host.rect(c));
    });

    test("windows one cell across can't be split that way", async () => {
        const host = await setup();
        const a = host.open({ defaultWidth: { fixed: 30 } });
        const b = host.open();
        host.activate(a);
        host.action("split-h");
        host.activate(b);
        const before = tiles(host);
        assert.equal(host.action("split-h"), false);
        assert.deepEqual(tiles(host), before);
    });
});

describe("layout API", () => {
    test("declares the gravity option", async () => {
        const host = await setup();
        assert.equal(host.meta.name, "bento");
        assert.deepEqual(host.meta.actions, ["split-h", "split-v", "swap"]);
        assert.deepEqual(host.meta.options, { gravity: "boolean" });
    });

    test("unknown actions do nothing", async () => {
        const host = await setup();
        host.open();
        assert.equal(host.action("no-such-action"), false);
        assert.equal(host.action("toString"), false);
    });

    test("focus actions return false at the edges for bento to fall back", async () => {
        const host = await setup();
        host.open();
        host.open();
        assert.equal(host.action("focus-column-right"), false);
        assert.equal(host.action("focus-column-left"), true);
        assert.equal(host.action("focus-column-left"), false);
        assert.equal(host.action("focus-window-or-monitor-up"), false);
        assert.equal(host.action("move-column-left-or-to-monitor-left"), false);
        assert.equal(host.action("focus-column-right-or-first"), true);
        assert.equal(host.focused, 2);
        assert.equal(host.action("focus-column-right-or-first"), true);
        assert.equal(host.focused, 1);
    });

    test("focus-column and move-column-to-index number the grid's columns", async () => {
        const host = await setUpTwoOverOne();
        host.activate(1);
        host.action("focus-column", 2);
        assert.equal(host.focused, 2);

        // Indices past the end give the last column.
        host.activate(1);
        host.action("focus-column", 9);
        assert.equal(host.focused, 2);

        // Window 2 moves to the first column, pushing window 1 along in its rows.
        host.activate(2);
        assert.equal(host.action("move-column-to-index", 1), true);
        assert.deepEqual(tiles(host), [
            "99 × 336 at x:131 y:16",
            "99 × 336 at x:16 y:16",
            "99 × 336 at x:16 y:368",
        ]);
    });

    test("focus-window-in-column counts the stack", async () => {
        const host = await setUpOneAndStack();
        host.activate(2);
        host.action("focus-window-in-column", 2);
        assert.equal(host.focused, 3);
        host.action("focus-window-down-or-top");
        assert.equal(host.focused, 2);
        host.action("focus-window-up-or-bottom");
        assert.equal(host.focused, 3);
    });

    test("a new window opens next to the window it belongs to", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.open();
        host.open({ nextTo: 1 });
        assert.deepEqual(
            [1, 4, 2, 3].map((id) => host.screenRect(id).x),
            [16, 131, 246, 361],
        );
    });

    test("windows opened without focus don't take it", async () => {
        const host = await setup();
        host.open();
        host.open({ activate: false });
        assert.equal(host.focused, 1);
    });

    test("the camera only scrolls to keep the focused window in view", async () => {
        const host = await setup();
        for (let i = 0; i < 12; i++) host.open();

        // The last window is in view at the right edge.
        assert.deepEqual(tiles(host, [12]), ["99 × 688 at x:1165 y:16"]);
        const camera = host.camera.x;

        // Closing a window to the left doesn't move the view; the windows slide instead.
        host.close(11);
        assert.equal(host.camera.x, camera);
        assert.deepEqual(tiles(host, [12]), ["99 × 688 at x:1050 y:16"]);

        // Focusing a window out of view scrolls to it.
        host.activate(1);
        assert.deepEqual(tiles(host, [1]), ["99 × 688 at x:16 y:16"]);
    });

    test("maximized windows fill the working area and push the others", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.activate(1);
        assert.equal(host.action("maximize-window-to-edges"), true);
        assert.equal(host.mode(1), "maximized");
        assert.deepEqual(tiles(host), ["1280 × 720 at x:0 y:0", "99 × 688 at x:1321 y:16"]);

        host.action("maximize-window-to-edges");
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 688 at x:131 y:16"]);
    });

    test("fullscreen-window toggles fullscreen", async () => {
        const host = await setup();
        host.open();
        assert.equal(host.action("fullscreen-window"), true);
        assert.equal(host.mode(1), "fullscreen");
        assert.deepEqual(tiles(host), ["1280 × 720 at x:0 y:0"]);
        host.action("fullscreen-window");
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16"]);
    });

    test("tabbed areas get a tab bar, which switches tabs", async () => {
        const host = await setUpStackOfTwo();
        host.activate(1);
        host.action("toggle-column-tabbed-display");

        const [bar] = host.tabs.values();
        assert.deepEqual(bar.windows, [1, 2]);
        assert.equal(bar.active, 1);
        assert.deepEqual(bar.area, { x: 0, y: 16, w: 99, h: 688 });

        host.clickTab(bar.key, 1);
        assert.equal(host.focused, 2);
        assert.equal(host.tabs.get(bar.key).active, 2);
        assert.equal(host.isVisible(2), true);
        assert.equal(host.isVisible(1), false);

        // No tab bar while the area covers the view.
        host.requestMode(2, "fullscreen");
        assert.equal(host.tabs.size, 0);
        host.requestMode(2, "normal");
        assert.equal(host.tabs.size, 1);

        // Moving tabs reorders them.
        host.action("move-window-up");
        assert.deepEqual(host.tabs.get(bar.key).windows, [2, 1]);

        // Closing a tab keeps the area's cells.
        host.close(2);
        assert.deepEqual(tiles(host, [1, 3]), ["99 × 688 at x:16 y:16", "99 × 688 at x:131 y:16"]);
        assert.equal(host.focused, 1);
    });

    test("tabs leave room for a tab indicator placed within the area", async () => {
        const host = await setup({
            config: {
                tabIndicator: {
                    off: false,
                    position: "left",
                    width: 4,
                    gap: 5,
                    placeWithinColumn: true,
                    hideWhenSingleTab: false,
                },
            },
        });
        host.open();
        host.open();
        host.activate(1);
        host.action("toggle-column-tabbed-display");
        assert.deepEqual(tiles(host), ["90 × 688 at x:25 y:16", "99 × 688 at x:131 y:16"]);
    });

    test("new windows are asked to open at the size of their cells", async () => {
        const host = await setup();
        assert.deepEqual(host.newWindowSize({ rules: { defaultWidth: { fixed: 100 } } }), {
            w: 99,
            h: 688,
        });
        assert.deepEqual(
            host.newWindowSize({
                rules: { defaultWidth: { proportion: 0.5 }, defaultHeight: { proportion: 0.5 } },
            }),
            { w: 616, h: 336 },
        );
        assert.deepEqual(host.newWindowSize({ rules: { defaultWidth: null } }), { w: 0, h: 688 });
        assert.deepEqual(
            host.newWindowSize({
                rules: { defaultWidth: { fixed: 100 } },
                minSize: { w: 300, h: 0 },
            }),
            { w: 300, h: 688 },
        );
    });

    test("windows keep their place and size through a reload", async () => {
        const host = await setUpTwoOverOne();
        host.actionOn(2, "set-window-width", set(50));
        host.activate(2);
        host.open();
        host.action("consume-or-expel-window-left");
        host.action("toggle-column-tabbed-display");
        const before = { tiles: tiles(host), focused: host.focused, ipc: [...host.ipc].sort() };

        host.reload();
        assert.deepEqual(host.errors, []);
        assert.deepEqual(tiles(host), before.tiles);
        assert.equal(host.focused, before.focused);
        assert.deepEqual([...host.ipc].sort(), before.ipc);
        assert.equal(host.tabs.size, 1);

        // Closing the window opened last still goes back to where it was opened from.
        host.close(4);
        assert.equal(host.focused, 2);
    });

    test("windows from another layout snap to the cells under them", async () => {
        const host = await setup();
        const adopt = (rect, activate) => {
            const win = host.makeWindow({});
            win.size = { w: rect.w, h: rect.h };
            host.send({
                type: "add",
                window: host.info(win),
                how: { reason: "adopt", activate, rect, pointer: null, nextTo: null },
            });
        };
        // 11 cells by 12, and 6 cells by 5.
        adopt({ x: 10, y: 10, w: 600, h: 700 }, true);
        adopt({ x: 640, y: 20, w: 300, h: 300 }, false);
        assert.deepEqual(tiles(host), ["616 × 688 at x:16 y:16", "329 × 277 at x:648 y:16"]);
    });

    test("turning gravity off and on", async () => {
        const host = await setUpHole();
        host.open();
        halfHeight(host, 3);
        host.action("move-window-down");

        // Without gravity, windows stay where they're moved.
        host.setOptions({ gravity: false });
        repeat(5, () => host.action("move-window-down"));
        assert.deepEqual(tiles(host, [3]), ["99 × 336 at x:246 y:368"]);
        assert.equal(host.action("move-column-left"), true);
        assert.equal(host.action("move-column-left"), true);
        assert.deepEqual(tiles(host, [3]), ["99 × 336 at x:131 y:368"]);

        // The leftmost window can go left of column 0 without gravity, but moving it alone left
        // just shifts everything, which isn't a change.
        host.activate(1);
        assert.equal(host.action("move-column-left"), false);

        // Turning gravity back on slides windows left.
        host.activate(3);
        host.action("move-column-right");
        host.setOptions({ gravity: true });
        assert.deepEqual(tiles(host, [3]), ["99 × 336 at x:131 y:368"]);
    });

    test("sizes stay proportional on a view with different cells", async () => {
        const host = await setup();
        host.open();
        host.actionOn(1, "set-window-width", set(50));
        host.open();
        host.actionOn(2, "set-window-width", set(50));
        host.activate(1);
        assert.deepEqual(tiles(host), ["616 × 688 at x:16 y:16", "616 × 688 at x:648 y:16"]);

        // 28 columns across instead of 22.
        host.resizeView({ size: { w: 2560, h: 1080 } });
        assert.deepEqual(tiles(host), ["1256 × 1048 at x:16 y:16", "1256 × 1048 at x:1288 y:16"]);

        // Struts shrink the working area.
        host.config = { ...host.config, struts: { left: 0, right: 0, top: 100, bottom: 0 } };
        host.send({ type: "config", config: host.config, options: host.options });
        assert.equal(host.screenRect(1).y, 116);
    });

    test("a window that can't fit its cells keeps its place", async () => {
        const host = await setup();
        host.open({ minSize: { w: 300, h: 0 } });
        host.open();
        assert.deepEqual(host.windows.get(1).size, { w: 300, h: 688 });
        assert.deepEqual(tiles(host), ["99 × 688 at x:16 y:16", "99 × 688 at x:131 y:16"]);
        assert.deepEqual(host.errors, []);
    });

    test("the highlight on an empty workspace is where the window will land", async () => {
        const host = await setup();
        host.camera = { x: 5000, y: 0 };
        host.send({ type: "camera", camera: host.camera });
        const win = host.makeWindow({});
        const target = host.dropTarget(
            win.id,
            { x: 5600, y: 300 },
            { x: 5500, y: 20, w: 99, h: 336 },
        );
        assert.deepEqual(
            { x: target.x - host.camera.x, y: target.y, w: target.w, h: target.h },
            { x: 16, y: 16, w: 99, h: 336 },
        );
    });
});

// =================================================================================================
// The cell grid on its own
// =================================================================================================

const R = DEFAULT_ROWS;
const A = (col, row, cols, rows) => new Area(col, row, cols, rows);

/** A grid with `items` as `[key, area]` pairs, as they are. */
function grid(items) {
    const g = new Grid(R);
    g.items = items.map(([key, area]) => ({ key, area }));
    g.verifyInvariants();
    return g;
}

function gravityGrid(items) {
    const g = grid(items);
    g.setGravity(true);
    g.verifyInvariants();
    return g;
}

function area(g, key) {
    return g.area(key).toJSON();
}

const at = (col, row, cols, rows) => ({ col, row, cols, rows });

describe("grid", () => {
    test("insert after pushes right", () => {
        const g = new Grid(R);
        g.insertAfter(1, null, 6, R);
        g.insertAfter(2, 1, 6, R);
        g.insertAfter(3, 1, 4, R);
        g.verifyInvariants();

        assert.deepEqual(area(g, 1), at(0, 0, 6, R));
        assert.deepEqual(area(g, 3), at(6, 0, 4, R));
        assert.deepEqual(area(g, 2), at(10, 0, 6, R));
    });

    test("insert aligns with top of previous", () => {
        const g = grid([
            [1, A(0, 6, 6, 6)],
            [2, A(0, 0, 6, 6)],
        ]);
        g.insertAfter(3, 1, 6, 6);
        g.verifyInvariants();
        assert.deepEqual(area(g, 3), at(6, 6, 6, 6));
    });

    test("move right swaps full height windows", () => {
        const g = grid([
            [1, A(0, 0, 6, R)],
            [2, A(6, 0, 6, R)],
        ]);
        assert.equal(g.moveWindow(1, Right), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 2), at(0, 0, 6, R));
        assert.deepEqual(area(g, 1), at(6, 0, 6, R));
    });

    test("move right swaps different widths", () => {
        const g = grid([
            [1, A(0, 0, 6, R)],
            [2, A(6, 0, 3, R)],
        ]);
        assert.equal(g.moveWindow(1, Right), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 2), at(0, 0, 3, R));
        assert.deepEqual(area(g, 1), at(3, 0, 6, R));
    });

    test("move past end does nothing", () => {
        const g = grid([
            [1, A(0, 0, 6, R)],
            [2, A(6, 0, 6, R)],
        ]);
        assert.equal(g.moveWindow(2, Right), false);
        assert.equal(g.moveWindow(1, Left), false);
        assert.equal(g.moveWindow(1, Up), false);
        assert.equal(g.moveWindow(1, Down), false);
    });

    test("move into hole", () => {
        // 1 | 3
        // 2 | .
        const g = grid([
            [1, A(0, 0, 6, 6)],
            [2, A(0, 6, 6, 6)],
            [3, A(6, 0, 6, 6)],
        ]);
        assert.equal(g.moveWindow(3, Down), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 3), at(6, 1, 6, 6));
    });

    test("move left out of shared column", () => {
        // 2 | 1
        // 3 | .
        const g = grid([
            [2, A(0, 0, 6, 6)],
            [3, A(0, 6, 6, 6)],
            [1, A(6, 0, 6, 6)],
        ]);
        assert.equal(g.moveWindow(3, Left), true);
        g.verifyInvariants();

        // Positions are fixed, so 3 goes one column left of column 0 rather than everything else
        // moving right. (grid.rs still expects 3 at column 0, 2 at 1 and 1 at 7, from before
        // columns stopped being renumbered to start at the leftmost window.)
        assert.deepEqual(area(g, 3), at(-1, 6, 6, 6));
        assert.deepEqual(area(g, 2), at(0, 0, 6, 6));
        assert.deepEqual(area(g, 1), at(6, 0, 6, 6));
    });

    test("vertical swap", () => {
        const g = grid([
            [1, A(0, 0, 6, 4)],
            [2, A(0, 4, 6, 8)],
        ]);
        assert.equal(g.moveWindow(1, Down), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 2), at(0, 0, 6, 8));
        assert.deepEqual(area(g, 1), at(0, 8, 6, 4));
    });

    test("swap pushes windows inside the exchanged block", () => {
        // 1 is 2 rows tall; 2 is a full-height window next to it; 3 sits in 1's rows past 2.
        const g = grid([
            [1, A(0, 0, 4, 2)],
            [2, A(4, 0, 2, R)],
            [3, A(6, 0, 2, 2)],
            [4, A(0, 2, 4, 10)],
        ]);
        assert.equal(g.moveWindow(1, Right), true);
        g.verifyInvariants();
    });

    test("remove collapses empty columns only", () => {
        const g = grid([
            [1, A(0, 0, 6, R)],
            [2, A(6, 0, 6, 6)],
            [3, A(6, 6, 3, 6)],
            [4, A(12, 0, 6, R)],
        ]);

        // Removing 3 leaves a hole under 2, since 2 still covers those columns.
        g.remove(3);
        g.verifyInvariants();
        assert.deepEqual(area(g, 4), at(12, 0, 6, R));

        // Removing 2 empties its columns, which collapse.
        g.remove(2);
        g.verifyInvariants();
        assert.deepEqual(area(g, 4), at(6, 0, 6, R));
    });

    test("growing down pushes down", () => {
        const g = grid([
            [1, A(0, 0, 6, 4)],
            [2, A(0, 4, 6, 4)],
        ]);
        assert.equal(g.resize(1, 6, 6), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 2), at(0, 6, 6, 4));
    });

    test("pushed off the bottom goes right", () => {
        const g = grid([
            [1, A(0, 0, 6, 6)],
            [2, A(0, 6, 6, 6)],
        ]);
        assert.equal(g.resize(1, 6, R), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 1), at(0, 0, 6, R));
        assert.deepEqual(area(g, 2), at(6, 6, 6, 6));
    });

    test("growing right cascades", () => {
        const g = grid([
            [1, A(0, 0, 4, R)],
            [2, A(4, 0, 4, 6)],
            [3, A(8, 0, 4, R)],
            [4, A(4, 6, 4, 6)],
        ]);
        assert.equal(g.resize(1, 6, R), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 2), at(6, 0, 4, 6));
        assert.deepEqual(area(g, 4), at(6, 6, 4, 6));
        assert.deepEqual(area(g, 3), at(10, 0, 4, R));
    });

    test("neighbor zigzag", () => {
        // AAAAAAAA DDDD
        // BBBB CCCCCCCC
        const g = grid([
            [1, A(0, 0, 8, 6)], // A
            [4, A(8, 0, 4, 6)], // D
            [2, A(0, 6, 4, 6)], // B
            [3, A(4, 6, 8, 6)], // C
            [5, A(12, 0, 6, R)], // E, next full-height window
        ]);
        const n = (id, dir, anchor = null) => g.neighbor(id, dir, anchor);

        assert.equal(n(1, Right), 4);
        assert.equal(n(4, Right), 5);
        assert.equal(n(2, Right), 3);
        assert.equal(n(3, Right), 5);
        assert.equal(n(4, Left), 1);

        // Down from A depends on which column the user is travelling along.
        assert.equal(n(1, Down, 1), 2);
        assert.equal(n(1, Down, 6), 3);

        // Left from E depends on which row the user is travelling along.
        assert.equal(n(5, Left, 2), 4);
        assert.equal(n(5, Left, 8), 3);

        assert.equal(n(1, Left), null);
        assert.equal(n(1, Up), null);
    });

    test("neighbor prefers nearest over anchor", () => {
        // A full-height window with a short window next to it and another one further away at
        // the anchor's height: the adjacent one wins so that nothing is skipped.
        const g = grid([
            [1, A(0, 0, 6, R)],
            [2, A(6, 6, 6, 6)],
            [3, A(12, 0, 6, 6)],
        ]);
        assert.equal(g.neighbor(1, Right, 2), 2);
    });

    test("adjacent only finds windows that share lines", () => {
        const g = grid([
            [1, A(0, 0, 6, 6)],
            [2, A(6, 0, 6, 6)],
            [3, A(0, 6, 6, 6)],
        ]);
        assert.equal(g.adjacent(2, Down), null);
        assert.equal(g.neighbor(2, Down), 3);
        assert.equal(g.adjacent(1, Down), 3);
    });

    test("gravity closes holes left by closed windows", () => {
        // 1 | 2 | 4
        //   | 3 |
        const g = gravityGrid([
            [1, A(0, 0, 4, R)],
            [2, A(4, 0, 4, 6)],
            [3, A(4, 6, 4, 6)],
            [4, A(8, 0, 4, 6)],
        ]);

        // Closing 2 lets 4 slide left into its rows; 3 keeps its place.
        g.remove(2);
        g.verifyInvariants();
        assert.deepEqual(area(g, 3), at(4, 6, 4, 6));
        assert.deepEqual(area(g, 4), at(4, 0, 4, 6));
    });

    test("gravity keeps holes that nothing fits", () => {
        // A full-height window next to a half-height one can't slide into half the space.
        const g = gravityGrid([
            [1, A(0, 0, 4, 6)],
            [2, A(4, 0, 4, R)],
            [3, A(0, 6, 4, 6)],
        ]);
        g.remove(3);
        g.verifyInvariants();
        assert.deepEqual(area(g, 2), at(4, 0, 4, R));
    });

    test("gravity pulls neighbors when shrinking", () => {
        const g = gravityGrid([
            [1, A(0, 0, 6, R)],
            [2, A(6, 0, 6, R)],
        ]);
        assert.equal(g.resize(1, 4, R), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 2), at(4, 0, 6, R));
    });

    test("gravity makes horizontal moves into empty space do nothing", () => {
        // 1 | 2
        // 3 |
        const g = gravityGrid([
            [1, A(0, 0, 4, 6)],
            [3, A(0, 6, 4, 6)],
            [2, A(4, 0, 4, 6)],
        ]);
        assert.equal(g.moveWindow(2, Right), false);

        // Moving 3 right swaps nothing either, and it slides back.
        assert.equal(g.moveWindow(3, Right), false);
    });

    test("gravity makes column 0 a wall", () => {
        const g = gravityGrid([
            [1, A(0, 0, 4, R)],
            [2, A(4, 0, 4, R)],
        ]);
        // Pushing 1 left past column 0 doesn't happen at all.
        assert.equal(g.setArea(2, A(2, 0, 6, R), Left), false);
        assert.deepEqual(area(g, 1), at(0, 0, 4, R));
        assert.deepEqual(area(g, 2), at(4, 0, 4, R));

        // Without gravity, the grid goes on to the left.
        g.setGravity(false);
        assert.equal(g.setArea(2, A(2, 0, 6, R), Left), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 1), at(-2, 0, 4, R));
    });

    test("pushes keep windows in order", () => {
        // 5 | 6 | 2, with 2 full height, and 3 | 4 at the bottom. 2 comes first in the grid's own
        // order, so that order can't decide who goes first.
        const g = gravityGrid([
            [2, A(12, 0, 8, R)],
            [3, A(0, 8, 5, 4)],
            [4, A(5, 8, 7, 4)],
            [5, A(0, 0, 8, 5)],
            [6, A(8, 0, 4, 5)],
        ]);

        // 7 lands where 6 is, and is wide enough to reach into 2. It pushes 6, which pushes 2.
        g.insert(7, A(8, 1, 12, 3));
        g.verifyInvariants();
        assert.deepEqual(area(g, 7), at(8, 1, 12, 3));
        assert.deepEqual(area(g, 6), at(20, 0, 4, 5));
        assert.deepEqual(area(g, 2), at(24, 0, 8, R));
    });

    test("pinned window falls only when released", () => {
        // 1 | 2
        //   | 2
        const g = gravityGrid([
            [1, A(0, 0, 4, 4)],
            [2, A(4, 0, 4, R)],
        ]);
        g.setPinned(2);

        // Shrinking 2 from the top frees rows next to nothing, but it stays put while held.
        const held = A(4, 4, 4, 8);
        assert.equal(g.setArea(2, held, Up), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 2), held.toJSON());

        // Letting go drops it under 1.
        g.setPinned(null);
        g.verifyInvariants();
        assert.deepEqual(area(g, 2), at(0, 4, 4, 8));
        assert.deepEqual(area(g, 1), at(0, 0, 4, 4));
    });

    test("windows slide up to a pinned window", () => {
        const g = gravityGrid([
            [1, A(0, 0, 4, R)],
            [2, A(4, 0, 4, R)],
            [3, A(8, 0, 4, R)],
        ]);
        g.setPinned(2);
        assert.equal(g.resize(2, 2, R), true);
        g.verifyInvariants();
        assert.deepEqual(area(g, 3), at(6, 0, 4, R));
    });

    test("rescaling keeps order and rows", () => {
        const g = gravityGrid([
            [1, A(0, 0, 11, R)],
            [2, A(11, 0, 11, 6)],
            [3, A(11, 6, 11, 6)],
        ]);
        g.rescale(28 / 22, () => [14, 6]);
        g.verifyInvariants();
        assert.deepEqual(area(g, 1), at(0, 0, 14, 6));
        assert.deepEqual(area(g, 2), at(14, 0, 14, 6));
        assert.deepEqual(area(g, 3), at(0, 6, 14, 6));
    });

    test("metrics near square", () => {
        const m = new CellMetrics({ w: 1920, h: 1080 }, R, 16);
        assert.equal(m.colsPerView, 22);
        assert.ok(Math.abs(m.cellW / m.cellH - 1) < 0.05);

        // The cells exactly fill the view.
        const full = m.rect(A(0, 0, m.colsPerView, R));
        assert.ok(Math.abs(full.x + full.w + m.gap - 1920) < 1e-6);
        assert.ok(Math.abs(full.y + full.h + m.gap - 1080) < 1e-6);

        assert.equal(m.rowsForProportion(0.5), 6);
        assert.equal(m.rowsForProportion(1 / 3), 4);
        assert.equal(m.rowsForProportion(0.25), 3);
        assert.equal(m.rowsForProportion(0.33333), 4);

        // Complementary proportions fit in the view.
        assert.equal(m.colsForProportion(0.5), 11);
        assert.equal(m.colsForProportion(0.33333), 7);
        assert.equal(m.colsForProportion(0.66667), 14);

        // The test output.
        const test = new CellMetrics({ w: 1280, h: 720 }, R, 16);
        assert.equal(test.colsPerView, 22);
    });

    test("size helpers", () => {
        assert.equal(cellsForDelta(0.4, 1), 1);
        assert.equal(cellsForDelta(-0.4, 1), -1);
        assert.equal(cellsForDelta(2.5, 1), 3);
        assert.equal(cellsForDelta(0, 1), 0);
        assert.equal(scaleSpan(11, 22, 28), 14);
        assert.equal(scaleSpan(7, 22, 22), 7);
        assert.equal(nextPresetSpan(2, [7, 11, 14], true), 7);
        assert.equal(nextPresetSpan(14, [7, 11, 14], true), 7);
        assert.equal(nextPresetSpan(7, [7, 11, 14], false), 14);
        assert.equal(nextPresetSpan(12, [7, 11, 14], false), 11);
    });

    test("random operations keep invariants", () => {
        // A small deterministic PRNG, so failures reproduce.
        let seed = 0x9e3779b9;
        const random = () => {
            seed = (seed + 0x6d2b79f5) | 0;
            let t = seed;
            t = Math.imul(t ^ (t >>> 15), t | 1);
            t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
            return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
        };
        const int = (min, max) => min + Math.floor(random() * (max - min + 1));
        const dirs = [Left, Right, Up, Down];

        for (let run = 0; run < 300; run++) {
            const g = new Grid(R);
            g.setGravity(random() < 0.5);
            let nextId = 0;
            const steps = int(1, 60);

            for (let step = 0; step < steps; step++) {
                const ids = g.keys();
                const pick = () => (ids.length > 0 ? ids[int(0, ids.length - 1)] : null);
                const sizes = g.items.map(({ key, area }) => [key, area]);

                // The window being operated on, which is the only one allowed to change size.
                let target = null;
                const op = int(0, 11);
                if (op < 3) {
                    g.insertAfter(nextId++, pick(), int(1, 24), int(1, R));
                } else if (op < 4) {
                    const id = pick();
                    if (id !== null) g.remove(id);
                } else if (op < 8) {
                    const id = pick();
                    if (id !== null) g.moveWindow(id, dirs[int(0, 3)]);
                } else if (op < 10) {
                    target = pick();
                    if (target !== null) g.resize(target, int(1, 24), int(1, R));
                } else if (op < 11) {
                    target = pick();
                    const a = A(int(-2, 29), int(0, R - 1), int(1, 12), int(1, R));
                    if (target !== null) g.setArea(target, a, dirs[int(0, 3)]);
                } else {
                    g.setPinned(random() < 0.3 ? null : pick());
                }

                g.verifyInvariants();

                // Pushing never resizes other windows.
                for (const [id, old] of sizes) {
                    const now = g.area(id);
                    if (id !== target && now !== null) {
                        assert.deepEqual([now.cols, now.rows], [old.cols, old.rows]);
                    }
                }
            }
        }
    });
});
