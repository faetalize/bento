// Tests for bento's grid layout engine, `bento.js`.
//
// The first part tests the engine through the layout API. The output is 1280 × 720 with 16 px
// gaps, which makes the grid 24 columns by 12 rows of roughly 37 × 43 px cells. Windows open
// 100 px wide, from `default-width`, so 2 cells wide. Expectations are in cells, so they
// don't depend on the output size: `cells()` gives windows' sizes and places on screen in cells,
// and `cellRect()` the rect of some cells on the canvas, rounded like the engine does. `tiles()`
// keeps pixels for windows that cover the view instead of cells, like fullscreen ones.
//
// The second part tests the grid arithmetic on its own.

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

const METRICS = new CellMetrics({ w: 1280, h: 720 }, DEFAULT_ROWS, 16);
/** Distance between the left edges of neighboring grid columns in the test output. */
const COL_STEP = METRICS.cellW + METRICS.gap;
/** Distance between the top edges of neighboring grid rows in the test output. */
const ROW_STEP = METRICS.cellH + METRICS.gap;

async function setup(opts = {}) {
    return TestHost.load("../bento.js", {
        ...opts,
        options: { "default-width": "100", ...(opts.options ?? {}) },
    });
}

/** Rounds halves away from zero, like bento rounds render positions to pixels. */
function px(value) {
    return Math.sign(value) * Math.round(Math.abs(value)) + 0;
}

/** Tiles in the order they opened, on screen. */
function tiles(host, ids = [...host.windows.keys()]) {
    return ids
        .sort((a, b) => a - b)
        .map((id) => {
            const r = host.screenRect(id);
            return `${px(r.w)} × ${px(r.h)} at x:${px(r.x)} y:${px(r.y)}`;
        });
}

/**
 * Tiles in the order they opened, in cells on screen: their size, then their column and row
 * relative to the view.
 */
function cells(host, ids = [...host.windows.keys()]) {
    const at = (value, step) => Number(((value - METRICS.gap) / step).toFixed(1));
    const span = (size, step) => Math.round((size + METRICS.gap) / step);
    return ids
        .sort((a, b) => a - b)
        .map((id) => {
            const r = host.screenRect(id);
            const size = `${span(r.w, COL_STEP)} × ${span(r.h, ROW_STEP)}`;
            return `${size} at ${at(r.x, COL_STEP)}, ${at(r.y, ROW_STEP)}`;
        });
}

/**
 * Canvas rect of `cols` × `rows` cells from column `col` and row `row`, with the edges rounded like
 * bento does, so that neighbors line up.
 */
function cellRect(col, row, cols, rows) {
    const x = px(col * COL_STEP);
    const top = METRICS.gap + row * ROW_STEP;
    const y = px(top);
    return {
        x,
        y,
        w: px((col + cols) * COL_STEP) - METRICS.gap - x,
        h: px(top + rows * METRICS.cellH + (rows - 1) * METRICS.gap) - y,
    };
}

const set = (value) => ({ op: "set", value, unit: "%" });
const adjust = (value) => ({ op: "adjust", value, unit: "%" });
/** A proportion that gives exactly `n` rows. */
const rows = (n) => set((n * 100) / 12 + 0.1);

function halfHeight(host, id) {
    host.actionOn(id, "set-height", set(50));
}

function repeat(n, fn) {
    for (let i = 0; i < n; i++) fn();
}

function edges(given) {
    return { left: false, right: false, top: false, bottom: false, ...given };
}

/** Starts a pointer resize; returns whether the engine took it. */
function resizeBegin(host, id, given) {
    const result = host.send({ type: "resizebegin", id, edges: edges(given), delta: { x: 0, y: 0 } });
    return result.handled && !result.prevented;
}

function resizeUpdate(host, id, given, dx, dy) {
    host.send({ type: "resizeupdate", id, edges: edges(given), delta: { x: dx, y: dy } });
}

function resizeEnd(host, id) {
    host.send({ type: "resizeend", id });
}

/** What the engine draws on a workspace: the boxes, in drawing order. */
function drawn(host, workspace = host.activeWorkspace().id) {
    const tree = host.tree(workspace);
    return tree
        .order()
        .filter((id) => tree.node(id).kind === "box")
        .map((id) => ({ id, ...tree.node(id).props }));
}

/** The boxes that clip the windows of groups, active or not. */
const clips = (host, workspace) => drawn(host, workspace).filter((node) => node.clip);

/** The slits of groups' windows that aren't active, which are the clips that take the pointer. */
const slits = (host, workspace) => clips(host, workspace).filter((node) => node.interactive);

/** The node that a window's node is in. */
const parentOf = (host, id, workspace) => host.tree(workspace).node(`w${id}`).parent;

/** The targets drawn for stuffing or a drag, which are in the hint color without its alpha. */
const targets = (host, workspace) => drawn(host, workspace).filter((node) => node.background === "#7fc8ff");

/** The target picked for stuffing, or under a dragged window's pointer: the one that stands out. */
const currentTarget = (host) => targets(host).find((node) => node.opacity === 1) ?? null;

const middle = (rect) => ({ x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 });

/** The highlights in the hint color: picked windows, and where a dragged window lands. */
const hints = (host) => drawn(host).filter((node) => node.background === "#7fc8ff80");

/** Picks windows `ids` with the group action while holding `mods`, then lets go of them. */
function pickToGroup(host, ids, mods = { super: true }) {
    host.setModifiers(mods);
    for (const id of ids) {
        host.activate(id);
        host.action("group");
    }
    host.setModifiers({});
}

/**
 * Drags window `id` with the pointer to `pointer` on the canvas and drops it there. Returns what
 * was drawn while it was over there: the targets, the one it was on, and the landing highlight.
 */
function dragOnto(host, id, pointer) {
    const r = host.rect(id);
    const rect = { x: pointer.x - 10, y: pointer.y - 10, w: r.w, h: r.h };
    const info = lift(host, id);
    const workspace = host.activeWorkspace().id;
    host.send({ type: "dragenter", workspace, window: info, pointer, rect });
    const shown = { targets: targets(host), current: currentTarget(host), landing: hint(host) };
    host.send({ type: "dragleave", workspace });
    drop(host, id, pointer, rect);
    return shown;
}

/** The highlight of a picked window or a drop target, if one is shown. */
const hint = (host, workspace) =>
    drawn(host, workspace).find((node) => node.background === "#7fc8ff80" && !node.interactive)
        ?.rect ?? null;

/** The state the engine exposes for the active workspace. */
function exposed(host) {
    const id = host.activeWorkspace().id;
    return host.exposedState().workspaces.find((ws) => ws.id === id);
}

/** The windows of each group. */
const groupedAreas = (host) =>
    exposed(host)
        .items.filter((item) => item.grouped)
        .map((item) => item.windows);

/**
 * Each window's column and place in its stack, counting from 1: the grid's columns are where
 * windows start, and a window's stack is the windows and groups covering the column of cells it
 * starts in. Sorted by window.
 */
function ipc(host) {
    const { items } = exposed(host);
    const starts = [...new Set(items.map((item) => item.area.col))].sort((a, b) => a - b);
    const stack = (col) =>
        items
            .filter((item) => item.area.col <= col && col < item.area.col + item.area.cols)
            .sort((a, b) => a.area.row - b.area.row)
            .flatMap((item) => item.windows.map((id) => [item, id]));
    return items
        .flatMap((item) =>
            item.windows.map((id) => [
                id,
                [
                    starts.indexOf(item.area.col) + 1,
                    stack(item.area.col).findIndex(([it, other]) => it === item && other === id) + 1,
                ],
            ]),
        )
        .sort((a, b) => a[0] - b[0]);
}

/** Lifts a window out of the layout for a drag, or because it floats. Returns what bento tells. */
function lift(host, id, reason = "drag") {
    const win = host.windows.get(id);
    const info = host.windowInfo(win);
    host.close(id, reason);
    host.windows.set(id, win);
    return info;
}

/** Gives a lifted window back to the engine, on the active workspace. */
function readd(host, id, how) {
    const win = host.windows.get(id);
    const workspace = host.activeWorkspace().id;
    win.workspace = workspace;
    host.trees.get(workspace).add(`w${id}`);
    host.told.windows.set(id, host.windowInfo(win));
    host.send({ type: "windowadd", window: host.windowInfo(win), how });
}

/** The highlight for a window dragged to `rect` with the pointer at `pointer`. */
function dropTarget(host, id, pointer, rect, info = host.windowInfo(host.windows.get(id))) {
    const workspace = host.activeWorkspace().id;
    host.send({ type: "dragenter", workspace, window: info, pointer, rect });
    const target = hint(host);
    host.send({ type: "dragleave", workspace });
    return target;
}

/** Drops a lifted window at `rect`, with the pointer at `pointer`. */
function drop(host, id, pointer, rect) {
    readd(host, id, { reason: "drop", activate: true, rect, pointer, nextTo: null });
}

/** Drags a window grabbed at `start` on screen and drops it with the pointer at `end`. */
function drag(host, id, start, end) {
    const camera = host.camera();
    const r = host.rect(id);
    const pointer = { x: end.x + camera.x, y: end.y + camera.y };
    const rect = { x: r.x + end.x - start.x, y: r.y + end.y - start.y, w: r.w, h: r.h };
    const info = lift(host, id);
    dropTarget(host, id, pointer, rect, info);
    drop(host, id, pointer, rect);
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
    repeat(6, () => host.action("move-down"));
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
    repeat(6, () => host.action("move-down"));
    return host;
}

/** Two windows of `width` percent stacked on the left, then a full-height one, focused. */
async function setUpStackAndOne(width, opts = {}) {
    const host = await setup(opts);
    for (const id of [1, 2, 3]) {
        host.open();
        host.actionOn(id, "set-width", set(width));
    }
    halfHeight(host, 1);
    halfHeight(host, 2);
    host.activate(2);
    repeat(6, () => host.action("move-down"));
    host.action("move-left");
    host.activate(3);
    return host;
}

/** Windows 1 and 2 stacked on the left, window 3 on the right. */
async function setUpStackOfTwo() {
    const host = await setup();
    host.open();
    host.open();
    host.open();
    host.activate(1);
    host.action("stack-from-right");
    host.activate(2);
    return host;
}

describe("grid layout", () => {
    test("new windows open to the right", async () => {
        const host = await setup();
        host.open();
        host.open();
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 12 at 2, 0"]);
        assert.equal(host.focused(), 2);
        assert.deepEqual(host.errors, []);
    });

    test("shrinking height leaves a hole", async () => {
        const host = await setUpHole();
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 6 at 2, 0"]);
    });

    test("moving down into a hole goes one cell at a time", async () => {
        const host = await setUpHole();
        host.action("move-down");
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 6 at 2, 1"]);

        // Moving past the bottom does nothing.
        repeat(10, () => host.action("move-down"));
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 6 at 2, 6"]);
        host.action("move-down");
    });

    test("moving into a window swaps", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.action("move-left");

        // Window 2 is now on the left.
        assert.deepEqual(cells(host), ["2 × 12 at 2, 0", "2 × 12 at 0, 0"]);
        assert.equal(host.focused(), 2);

        // Column 0 is the left edge of the grid with gravity.
        host.action("move-left");
    });

    test("growing down takes rows from the window stacked below", async () => {
        const host = await setUpHole();

        // Put window 3 below window 2 by opening it and moving it down. Left gravity slides it
        // under window 2.
        host.open();
        halfHeight(host, 3);
        repeat(6, () => host.action("move-down"));
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 6 at 2, 0", "2 × 6 at 2, 6"]);

        // Focus moves geometrically.
        host.action("focus-up");
        assert.equal(host.focused(), 2);
        host.action("focus-down");
        assert.equal(host.focused(), 3);
        host.action("focus-left");
        assert.equal(host.focused(), 1);

        // Growing 2 down moves the edge it shares with 3, which gets shorter instead of being
        // pushed away.
        host.actionOn(2, "set-height", set(75));
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 9 at 2, 0", "2 × 3 at 2, 9"]);
    });

    test("closing a window collapses empty columns", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.open();
        host.close(2);

        // Window 3 slides into the place of window 2, and the view stays still.
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 12 at 2, 0"]);
    });

    test("switching preset width snaps to cells", async () => {
        const host = await setup();
        host.open();

        // From 2 cells to the next larger preset, 1/3 (8 cells).
        host.action("cycle-width");
        assert.deepEqual(cells(host), ["8 × 12 at 0, 0"]);

        // Then 1/2 (12 cells).
        host.action("cycle-width");
        assert.deepEqual(cells(host), ["12 × 12 at 0, 0"]);

        // And back.
        host.action("cycle-width-back");
        assert.deepEqual(cells(host), ["8 × 12 at 0, 0"]);
    });

    test("dragging the right edge snaps and pushes", async () => {
        const host = await setup();
        host.open();
        host.open();
        assert.equal(resizeBegin(host, 1, { right: true }), true);

        // A bit over 2.5 cells snaps to 3 more cells.
        resizeUpdate(host, 1, { right: true }, COL_STEP * 2.6, 0);

        // Window 1 is 5 cells wide and pushed window 2 along.
        assert.deepEqual(cells(host), ["5 × 12 at 0, 0", "2 × 12 at 5, 0"]);

        // Dragging back puts window 2 back where it was.
        resizeUpdate(host, 1, { right: true }, COL_STEP * 0.4, 0);
        resizeEnd(host, 1);
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 12 at 2, 0"]);
    });

    test("dragging the bottom edge leaves a hole", async () => {
        const host = await setup();
        host.open();
        host.resize(1, { bottom: true }, { x: 0, y: -ROW_STEP * 4 });

        // 8 rows tall.
        assert.deepEqual(cells(host), ["2 × 8 at 0, 0"]);
    });

    test("dragging the left edge stops at the start of the grid", async () => {
        const host = await setup();
        host.open();
        host.open();
        resizeBegin(host, 2, { left: true });
        resizeUpdate(host, 2, { left: true }, -COL_STEP * 3, 0);

        // Window 1 is already at the start of the grid, so window 2 can't push it any further,
        // and doesn't grow.
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 12 at 2, 0"]);

        // Shrinking from the left still works, keeping the right edge still.
        resizeUpdate(host, 2, { left: true }, COL_STEP, 0);
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "1 × 12 at 3, 0"]);

        // Letting go drops it next to window 1.
        resizeEnd(host, 2);
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "1 × 12 at 2, 0"]);
    });

    test("dropping a window into a cell", async () => {
        // Window 1 full height, window 2 on top to its right, window 3 half height further right.
        const host = await setup();
        host.open();
        host.open();
        halfHeight(host, 2);
        host.open();
        halfHeight(host, 3);
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 6 at 2, 0", "2 × 6 at 4, 0"]);

        // Grab window 3 near its top-left corner and drop it into the hole under window 2.
        const start = { x: 16 + 4 * COL_STEP + 4, y: 16 + 4 };
        const end = { x: 16 + 2 * COL_STEP + 4, y: 16 + 6 * ROW_STEP + 4 };

        // The highlight shows where it lands.
        const r = host.rect(3);
        const pointer = { x: end.x + host.camera().x, y: end.y };
        const rect = { x: r.x + end.x - start.x, y: r.y + end.y - start.y, w: r.w, h: r.h };
        assert.deepEqual(dropTarget(host, 3, pointer, rect), cellRect(2, 6, 2, 6));

        drag(host, 3, start, end);
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 6 at 2, 0", "2 × 6 at 2, 6"]);
        assert.equal(host.focused(), 3);
    });

    test("dropping on the dot near an edge of a window splits it", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.open();

        // Window 1's dots are 40 px inside its edges, or a quarter of it if that's less. Drop
        // window 3 on the top one: 3 takes the top half and 1 keeps the bottom.
        let r = host.rect(1);
        const top = { x: r.x + r.w / 2, y: r.y + 40 };
        let shown = dragOnto(host, 3, top);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 6", "2 × 12 at 2, 0", "2 × 6 at 0, 0"]);

        // While over it, the dot stands out, larger and round, and nothing else is highlighted.
        assert.deepEqual(shown.current.rect, { x: px(top.x - 14), y: top.y - 14, w: 28, h: 28 });
        assert.equal(shown.current.cornerRadius, 14);
        assert.equal(shown.landing, null);
        assert.ok(shown.targets.length > 1);

        // Drop window 2 on window 1's left dot: 2 takes the left column, 1 the right one.
        r = host.rect(1);
        shown = dragOnto(host, 2, { x: r.x + r.w / 4, y: r.y + r.h / 2 });
        assert.deepEqual(cells(host), ["1 × 6 at 1, 6", "1 × 6 at 0, 6", "2 × 6 at 0, 0"]);
    });

    test("dropping on a seam stuffs the window into it", async () => {
        const host = await setup({ options: { "default-width": "50%" } });
        const a = host.open();
        const b = host.open();
        const w = host.open({ rules: { "default-width": { fixed: 100 } } });

        // The seam's pill is on the middle of the edge A and B share.
        const seam = { x: (cellRect(0, 0, 12, 12).w + cellRect(12, 0, 12, 12).x) / 2, y: 360 };
        const shown = dragOnto(host, w, seam);
        assert.deepEqual(middle(shown.current.rect), seam);
        assert.deepEqual(host.rect(a), cellRect(0, 0, 8, 12));
        assert.deepEqual(host.rect(w), cellRect(8, 0, 8, 12));
        assert.deepEqual(host.rect(b), cellRect(16, 0, 8, 12));
    });

    test("dropping between two windows of a group joins it there", async () => {
        const host = await setup({ options: { "default-width": "50%" } });
        const a = host.open();
        const b = host.open();
        const w = host.open();
        host.groupWindows([a, b], a);

        // A is active, and B a slit at the right of the area; the pill is between them.
        const area = cellRect(0, 0, 12, 12);
        const between = { x: area.x + area.w - 40 - 4, y: area.y + area.h / 2 };
        dragOnto(host, w, between);
        assert.deepEqual(host.groupOf(a).windows, [a, w, b]);
        assert.equal(host.groupOf(a).active, w);
        assert.equal(host.focused(), w);
    });

    test("dragging the bottom window of a stack keeps the view still", async () => {
        // Window 2 under window 1, focused.
        const host = await setup();
        host.open();
        halfHeight(host, 1);
        host.open();
        halfHeight(host, 2);
        repeat(6, () => host.action("move-down"));
        host.action("move-left");
        assert.equal(host.focused(), 2);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 0, 6"]);

        // Lift window 2 out of the layout. Window 1 hasn't jumped.
        lift(host, 2);
        assert.deepEqual(cells(host, [1]), ["2 × 6 at 0, 0"]);
        assert.equal(host.focused(), 1);
    });

    test("dragging a window that fell left keeps the view still", async () => {
        // Windows 1 and 2 on the top half, then window 3 opens right of window 2, and falls under
        // window 1 when moved to the bottom half.
        const host = await setUpTwoOverOne();
        assert.equal(host.focused(), 3);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 2, 0", "2 × 6 at 0, 6"]);

        // Windows 1 and 2 stay where they were.
        lift(host, 3);
        assert.deepEqual(cells(host, [1, 2]), ["2 × 6 at 0, 0", "2 × 6 at 2, 0"]);
    });

    test("closing a window keeps focus on another", async () => {
        const host = await setUpTwoOverOne();
        host.activate(2);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 2, 0", "2 × 6 at 0, 6"]);

        // Closing window 1 lets window 2 slide left, which reorders the windows.
        host.close(1);
        assert.equal(host.focused(), 2);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 0, 6"]);
    });

    test("closing the focused window focuses the one above", async () => {
        const host = await setUpTwoOverOne();
        // Focus away and back so that closing window 3 doesn't go back to window 2 it was opened
        // from.
        host.activate(1);
        host.activate(3);
        host.close(3);
        assert.equal(host.focused(), 1);
    });

    test("closing a new window goes back to the previous one", async () => {
        // Window 1 on top of window 2, both on the left.
        const host = await setup();
        host.open();
        halfHeight(host, 1);
        host.open();
        halfHeight(host, 2);
        repeat(6, () => host.action("move-down"));
        host.action("move-left");
        host.activate(1);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 0, 6"]);

        // Window 3 opens right of window 1, after window 2 in the order.
        host.open();
        assert.equal(host.focused(), 3);

        host.close(3);
        assert.equal(host.focused(), 1);
    });

    test("swiping snaps to window edges with stacked windows", async () => {
        // Windows 1 and 2 stacked on the left, then windows 3 and 4, all half the view wide.
        const host = await setup();
        for (const id of [1, 2, 3, 4]) {
            host.open();
            host.actionOn(id, "set-width", set(50));
        }
        halfHeight(host, 1);
        halfHeight(host, 2);
        host.activate(2);
        repeat(6, () => host.action("move-down"));
        host.action("move-left");
        host.activate(1);
        assert.deepEqual(cells(host), [
            "12 × 6 at 0, 0",
            "12 × 6 at 0, 6",
            "12 × 12 at 12, 0",
            "12 × 12 at 24, 0",
        ]);

        // Swipe right by a bit over one window.
        host.swipe(host.activeWorkspace().id, [700]);

        // The view snaps to window 3 on the left and window 4 on the right, focusing window 4 as
        // the furthest in the direction of the swipe.
        assert.equal(host.focused(), 4);
        assert.deepEqual(cells(host), [
            "12 × 6 at -12, 0",
            "12 × 6 at -12, 6",
            "12 × 12 at 0, 0",
            "12 × 12 at 12, 0",
        ]);
    });

    test("swiping snaps where the swipe would coast to", async () => {
        const host = await setup();
        for (const id of [1, 2, 3, 4]) {
            host.open();
            host.actionOn(id, "set-width", set(50));
        }
        host.activate(1);

        // A small swipe that would coast far snaps to the far end.
        host.swipe(host.activeWorkspace().id, [100], 3000);
        assert.equal(host.focused(), 4);
        assert.deepEqual(cells(host, [3, 4]), ["12 × 12 at 0, 0", "12 × 12 at 12, 0"]);
    });

    test("stacked windows count as a single column for centering", async () => {
        const host = await setup({ options: { "always-center-single-window": true } });
        host.open();
        halfHeight(host, 1);
        host.open();
        halfHeight(host, 2);
        repeat(6, () => host.action("move-down"));
        host.action("move-left");
        assert.deepEqual(cells(host), ["2 × 6 at 11, 0", "2 × 6 at 11, 6"]);
    });

    test("centering visible columns uses their span on the grid", async () => {
        const host = await setUpStackAndOne(20);
        host.action("center-visible");
        assert.deepEqual(cells(host), ["4 × 6 at 8, 0", "4 × 6 at 8, 6", "4 × 12 at 12, 0"]);
    });

    test("center on overflow compares with the window beside", async () => {
        const host = await setUpStackAndOne(60, { options: { "center-focused-window": "on-overflow" } });
        host.action("focus-left");
        assert.equal(host.focused(), 1);
        // Window 1 and window 3 beside it don't fit together, so window 1 is centered. Window 2
        // under it is in the same column of cells and doesn't count.
        assert.deepEqual(cells(host), ["14 × 6 at 5, 0", "14 × 6 at 5, 6", "14 × 12 at 19, 0"]);
    });

    test("toggling full width restores the previous width", async () => {
        const host = await setup();
        host.open();
        host.actionOn(1, "set-width", set(33));
        host.action("toggle-full-width");
        assert.deepEqual(cells(host), ["24 × 12 at 0, 0"]);

        host.action("toggle-full-width");
        assert.deepEqual(cells(host), ["7 × 12 at 0, 0"]);
    });

    test("adjusting width goes by the requested amount", async () => {
        // 10% of 24 columns is 2 columns.
        const host = await setup();
        host.open();
        host.actionOn(1, "set-width", adjust(10));
        assert.deepEqual(cells(host), ["4 × 12 at 0, 0"]);

        // A small adjustment still changes the size by a cell.
        host.action("set-width", { op: "adjust", value: -10, unit: "px" });
        assert.deepEqual(cells(host), ["3 × 12 at 0, 0"]);
    });

    test("floating and tiling again keeps the size and place", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.actionOn(2, "set-width", set(33));
        host.actionOn(2, "set-height", set(50));
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "7 × 6 at 2, 0"]);

        // The window floats where it was tiled, then is tiled again from there.
        const rect = host.rect(2);
        lift(host, 2, "float");
        readd(host, 2, { reason: "tile", activate: true, rect, pointer: null, nextTo: null });
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "7 × 6 at 2, 0"]);
    });

    test("default window height opens next to the active window", async () => {
        // Window 1 in the bottom half.
        const host = await setup();
        host.open();
        halfHeight(host, 1);
        repeat(6, () => host.action("move-down"));

        // Window 2 opens half height, lined up with window 1.
        host.open({ rules: { "default-height": { proportion: 0.5 } } });
        assert.deepEqual(cells(host), ["2 × 6 at 0, 6", "2 × 6 at 2, 6"]);
    });

    test("focusing the last column stays in the row", async () => {
        const host = await setUpOneAndStack();
        host.activate(1);
        host.action("focus-last");
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 6 at 2, 0", "2 × 6 at 2, 6"]);

        // Window 3 comes last in the order, but window 2 is the last one in the top row.
        assert.equal(host.focused(), 2);
    });

    test("focusing the top window goes up the stack", async () => {
        const host = await setUpOneAndStack();
        host.activate(3);
        host.action("focus-stack-top");
        assert.equal(host.focused(), 2);
    });

    test("focusing down reaches windows diagonally", async () => {
        // Window 3 is under window 1, not under window 2.
        const host = await setUpTwoOverOne();
        host.activate(2);

        host.action("focus-down");
        assert.equal(host.focused(), 3);
    });

    test("ipc positions follow grid columns", async () => {
        const host = await setUpTwoOverOne();

        // Windows 1 and 3 are stacked in the first column, window 2 is in the second.
        assert.deepEqual(
            ipc(host),
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
        repeat(6, () => host.action("move-down"));
        host.activate(2);
        host.requestMode(2, "fullscreen");

        // Window 2 covers the view, and window 3 is pushed past it rather than drawn under it.
        assert.deepEqual(tiles(host, [2]), ["1280 × 720 at x:0 y:0"]);
        assert.deepEqual(cells(host, [1, 3]), ["2 × 12 at -2.3, 0", "2 × 6 at 24.7, 6"]);
        assert.equal(host.mode(2), "fullscreen");

        // Everything goes back once it ends.
        host.requestMode(2, "normal");
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 6 at 2, 0", "2 × 6 at 2, 6"]);
        assert.equal(host.mode(2), "normal");
    });

    test("expanding fills the free cells in view", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.action("fill-width");

        // Window 2 grows up to the right edge of the view, and window 1 stays as is.
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "22 × 12 at 2, 0"]);
        host.action("fill-width");
    });

    test("stacking splits rows and unstacking gives them back", async () => {
        const host = await setUpStackOfTwo();

        // Window 2 goes under window 1, which keeps the top half, and window 3 slides left.
        assert.equal(host.focused(), 2);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 0, 6", "2 × 12 at 2, 0"]);

        // Window 2 is stacked, and goes back out to the right.
        host.action("unstack");
        assert.equal(host.focused(), 2);
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 12 at 2, 0", "2 × 12 at 4, 0"]);
    });

    test("consuming into column brings the window on the right under", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.activate(1);
        host.action("stack-from-right");
        assert.equal(host.focused(), 1);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 0, 6"]);

        // Expelling the focused window gives window 2 its rows back.
        host.action("unstack");
        assert.equal(host.focused(), 1);
        assert.deepEqual(cells(host), ["2 × 12 at 2, 0", "2 × 12 at 0, 0"]);
    });

    test("windows picked with the group key group where the first one is, once the keys are let go", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.open();
        host.setModifiers({ super: true });
        host.activate(3);
        host.action("group");
        host.activate(1);
        host.action("group");

        // Nothing's grouped while the keys are held, and the picked windows are highlighted.
        assert.equal(host.groupOf(1), null);
        assert.deepEqual(hints(host).map((node) => node.rect), [host.rect(3), host.rect(1)]);

        host.setModifiers({});
        assert.deepEqual(hints(host), []);
        assert.deepEqual(
            exposed(host).items.map((item) => [item.windows, item.area]),
            [
                [[2], at(0, 0, 2, 12)],
                [[3, 1], at(2, 0, 2, 12)],
            ],
        );
        assert.equal(host.focused(), 1);

        // Focusing left and right goes through the group's windows, then on to the grid.
        host.action("focus-left");
        assert.equal(host.focused(), 3);
        host.action("focus-left");
        assert.equal(host.focused(), 2);
        host.action("focus-right");
        assert.equal(host.focused(), 3);
    });

    test("a window picked alone leaves its group", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.groupWindows([1, 2], 1);
        pickToGroup(host, [2]);
        assert.equal(host.groupOf(2), null);
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 12 at 2, 0"]);
    });

    test("picking a window again unpicks it", async () => {
        const host = await setup();
        host.open();
        host.open();
        pickToGroup(host, [1, 2, 2]);
        assert.equal(host.groupOf(1), null);
        assert.deepEqual(hints(host), []);
    });

    test("without keys held, the group action acts at once", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.groupWindows([1, 2], 1);
        host.activate(2);
        host.action("group");
        assert.equal(host.groupOf(2), null);
    });

    test("picking into a group adds to it, and windows leave the groups they were in", async () => {
        const host = await setup();
        for (let i = 0; i < 4; i++) host.open();
        host.groupWindows([1, 2], 1);
        host.groupWindows([3, 4], 3);
        pickToGroup(host, [1, 4]);
        assert.deepEqual(groupedAreas(host), [[1, 2, 4]]);
        assert.equal(host.groupOf(3), null);
    });

    test("stacking under a group stacks the window, which doesn't join the group", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.open();
        host.groupWindows([1, 2], 1);
        host.activate(1);
        host.action("stack-from-right");
        assert.deepEqual(groupedAreas(host), [[1, 2]]);
        assert.deepEqual(
            exposed(host).items.map((item) => [item.windows, item.area]),
            [
                [[1, 2], at(0, 0, 2, 6)],
                [[3], at(0, 6, 2, 6)],
            ],
        );
    });
});

describe("split and swap", () => {
    /** A and B stacked on the left half, C on the right half at full height. */
    async function setUpStackAndOne() {
        const host = await setup({ options: { "default-width": "50%" } });
        const a = host.open();
        const b = host.open();
        host.activate(a);
        host.action("stack-from-right");
        host.activate(b);
        const c = host.open();
        assert.deepEqual(cells(host), ["12 × 6 at 0, 0", "12 × 6 at 0, 6", "12 × 12 at 12, 0"]);
        return { host, a, b, c };
    }

    test("splitting side by side gives the window the half facing it", async () => {
        const { host, a, c } = await setUpStackAndOne();

        // The first press picks A and highlights it.
        host.activate(a);
        host.action("split-h");
        assert.deepEqual(hint(host), cellRect(0, 0, 12, 6));

        // The second press moves C into A's right half.
        host.activate(c);
        host.action("split-h");
        assert.equal(hint(host), null);
        assert.deepEqual(cells(host), ["6 × 6 at 0, 0", "12 × 6 at 0, 6", "6 × 6 at 6, 0"]);
        assert.equal(host.focused(), c);
    });

    test("splitting the bottom window of a stack", async () => {
        const { host, b, c } = await setUpStackAndOne();
        host.activate(b);
        host.action("split-h");
        host.activate(c);
        host.action("split-h");
        assert.deepEqual(cells(host), ["12 × 6 at 0, 0", "6 × 6 at 0, 6", "6 × 6 at 6, 6"]);
    });

    test("splitting stacked gives the window the half facing it", async () => {
        const host = await setup({ options: { "default-width": "50%" } });
        const a = host.open();
        const b = host.open();
        host.action("split-v");
        host.activate(a);
        host.action("split-v");

        // A and B are level, so A goes below B, and the column it left collapses.
        assert.deepEqual(cells(host), ["12 × 6 at 0, 6", "12 × 6 at 0, 0"]);
        assert.equal(host.focused(), a);
        host.action("focus-up");
        assert.equal(host.focused(), b);
    });

    test("swapping trades places and sizes", async () => {
        const { host, a, c } = await setUpStackAndOne();
        host.activate(a);
        host.action("swap");
        host.activate(c);
        host.action("swap");
        assert.deepEqual(cells(host), ["12 × 12 at 12, 0", "12 × 6 at 0, 6", "12 × 6 at 0, 0"]);
    });

    test("picking the same window again cancels", async () => {
        const { host, a } = await setUpStackAndOne();
        host.activate(a);
        const before = tiles(host);
        host.action("swap");
        host.action("swap");
        assert.equal(hint(host), null);
        assert.deepEqual(tiles(host), before);
    });

    test("a picked window that closes ends the action", async () => {
        const { host, a, c } = await setUpStackAndOne();
        host.activate(a);
        host.action("split-h");
        host.close(a);
        assert.equal(hint(host), null);

        // The next press starts over.
        host.activate(c);
        host.action("split-h");
        assert.deepEqual(hint(host), host.rect(c));
    });

    test("windows one cell across can't be split that way", async () => {
        const host = await setup();
        const a = host.open({ rules: { "default-width": { fixed: 30 } } });
        const b = host.open();
        host.activate(a);
        host.action("split-h");
        host.activate(b);
        const before = tiles(host);
        host.action("split-h");
        assert.deepEqual(tiles(host), before);
    });
});

describe("sticky vertical resizing", () => {
    const set = (value) => ({ op: "set", value, unit: "%" });
    const row = 720 / 12 - 16 * (13 / 12) + 16;

    /** Window 1 on top of window 2, half each. */
    async function setUpStack() {
        const host = await setup();
        host.open();
        host.open();
        host.activate(1);
        host.action("stack-from-right");
        host.activate(2);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 0, 6"]);
        return host;
    }

    test("shrinking gives the rows to the window stacked below", async () => {
        const host = await setUpStack();
        host.actionOn(1, "set-height", set(25));
        assert.deepEqual(cells(host), ["2 × 3 at 0, 0", "2 × 9 at 0, 3"]);
    });

    test("the bottom window grows up, taking rows from the one above", async () => {
        const host = await setUpStack();
        host.actionOn(2, "set-height", set(75));
        assert.deepEqual(cells(host), ["2 × 3 at 0, 0", "2 × 9 at 0, 3"]);
    });

    test("the edge stops where the other window can't shrink any further", async () => {
        const host = await setUpStack();
        host.actionOn(1, "set-height", set(100));
        assert.deepEqual(cells(host), ["2 × 11 at 0, 0", "2 × 1 at 0, 11"]);
    });

    test("dragging an edge moves the window across it too", async () => {
        const host = await setUpStack();
        assert.equal(host.resize(1, { bottom: true }, { x: 0, y: 2 * row }), true);
        assert.deepEqual(cells(host), ["2 × 8 at 0, 0", "2 × 4 at 0, 8"]);

        const other = await setUpStack();
        assert.equal(other.resize(2, { top: true }, { x: 0, y: -2 * row }), true);
        assert.deepEqual(cells(other), ["2 × 4 at 0, 0", "2 × 8 at 0, 4"]);
    });

    /** Windows 1 and 2 side by side on the top half, window 3 under both on the bottom half. */
    async function setUpTwoOverWide() {
        const host = await setup();
        for (const id of [1, 2, 3]) {
            host.open();
            host.actionOn(id, "set-height", rows(6));
        }
        host.actionOn(3, "set-width", set((4 * 100) / METRICS.colsPerView + 0.1));
        repeat(6, () => host.action("move-down"));
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 2, 0", "4 × 6 at 0, 6"]);
        return host;
    }

    test("a narrower window sticks to the edge it lies along", async () => {
        const host = await setup();
        host.open({ rules: { "default-width": { fixed: 200 } } });
        host.open();
        host.activate(1);
        host.action("stack-from-right");
        host.activate(2);
        host.actionOn(2, "set-width", { op: "set", value: 100, unit: "px" });
        assert.deepEqual(cells(host), ["4 × 6 at 0, 0", "2 × 6 at 0, 6"]);

        host.actionOn(1, "set-height", rows(9));
        assert.deepEqual(cells(host), ["4 × 9 at 0, 0", "2 × 3 at 0, 9"]);
    });

    test("a window under two windows moves both", async () => {
        const host = await setUpTwoOverWide();
        host.actionOn(3, "set-height", rows(8));
        assert.deepEqual(cells(host), ["2 × 4 at 0, 0", "2 × 4 at 2, 0", "4 × 8 at 0, 4"]);
        host.actionOn(3, "set-height", rows(3));
        assert.deepEqual(cells(host), ["2 × 9 at 0, 0", "2 × 9 at 2, 0", "4 × 3 at 0, 9"]);
    });

    test("the edge stops where one of the stuck windows can't shrink any further", async () => {
        const host = await setUpTwoOverWide();
        host.setLimits(2, { minSize: { w: 0, h: 5 * ROW_STEP - 16 }, maxSize: { w: 0, h: 0 } });
        host.actionOn(3, "set-height", rows(10));
        assert.deepEqual(cells(host), ["2 × 5 at 0, 0", "2 × 5 at 2, 0", "4 × 7 at 0, 5"]);
    });

    test("a wider window isn't stuck: it gets pushed", async () => {
        // Window 3 can't go down past the bottom, so it goes right of window 1, keeping its rows.
        const host = await setUpTwoOverWide();
        host.actionOn(1, "set-height", rows(7));
        assert.deepEqual(cells(host), ["2 × 7 at 0, 0", "2 × 6 at 2, 0", "4 × 6 at 2, 6"]);

        // Shrinking leaves it where it is.
        const other = await setUpTwoOverWide();
        other.actionOn(1, "set-height", rows(4));
        assert.deepEqual(cells(other), ["2 × 4 at 0, 0", "2 × 6 at 2, 0", "4 × 6 at 0, 6"]);
    });

    test("a window on the bottom stays there", async () => {
        const host = await setUpTwoOverWide();
        // Its bottom edge doesn't move when dragged.
        host.resize(3, { bottom: true }, { x: 0, y: -2 * ROW_STEP });
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0", "2 × 6 at 2, 0", "4 × 6 at 0, 6"]);
        // Dragging its top edge moves the windows stuck to it.
        host.resize(3, { top: true }, { x: 0, y: -2 * ROW_STEP });
        assert.deepEqual(cells(host), ["2 × 4 at 0, 0", "2 × 4 at 2, 0", "4 × 8 at 0, 4"]);
    });

    test("a window as tall as the grid shrinks from the bottom", async () => {
        const host = await setup();
        host.open();
        host.actionOn(1, "set-height", rows(6));
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0"]);
    });
});

describe("stuff", () => {
    const half = { options: { "default-width": "50%" } };
    const rect = (x, y, w, h) => ({ x, y, w, h });

    test("stuffing between windows side by side splits their span in three", async () => {
        const host = await setup(half);
        const a = host.open();
        const b = host.open();
        const w = host.open({ rules: { "default-width": { fixed: 100 } } });

        // The first press shows the window, and the seam it would go into: its pill, on the
        // middle of the edge A and B share.
        host.action("stuff");
        assert.deepEqual(hint(host), host.rect(w));
        assert.deepEqual(
            targets(host).map((node) => middle(node.rect)),
            [{ x: (cellRect(0, 0, 12, 12).w + cellRect(12, 0, 12, 12).x) / 2, y: 360 }],
        );

        // The second press puts it there: 12 + 12 cells become 8, 8 and 8, and A and B keep
        // their outer edges.
        host.action("stuff");
        assert.equal(hint(host), null);
        assert.equal(targets(host).length, 0);
        assert.deepEqual(host.rect(a), cellRect(0, 0, 8, 12));
        assert.deepEqual(host.rect(w), cellRect(8, 0, 8, 12));
        assert.deepEqual(host.rect(b), cellRect(16, 0, 8, 12));
        assert.equal(host.focused(), w);
    });

    test("stuffing between stacked windows", async () => {
        const host = await setup(half);
        const a = host.open();
        const b = host.open();
        host.activate(a);
        host.action("stack-from-right");
        host.activate(b);
        const w = host.open();
        host.action("stuff");
        host.action("stuff");
        assert.deepEqual(host.rect(a), cellRect(0, 0, 12, 4));
        assert.deepEqual(host.rect(w), cellRect(0, 4, 12, 4));
        assert.deepEqual(host.rect(b), cellRect(0, 8, 12, 4));
    });

    test("the window takes the rows the two windows share", async () => {
        const host = await setup(half);
        const a = host.open();
        const b = host.open({ rules: { "default-height": { proportion: 0.5 } } });
        const w = host.open({ rules: { "default-width": { fixed: 100 } } });
        host.action("stuff");
        host.action("stuff");

        // A is full height and B half, so W gets the top half, and A shrinks at full height.
        assert.deepEqual(host.rect(a), cellRect(0, 0, 8, 12));
        assert.deepEqual(host.rect(w), cellRect(8, 0, 8, 6));
        assert.deepEqual(host.rect(b), cellRect(16, 0, 8, 6));
    });

    test("direction keys move between seams, and other actions cancel", async () => {
        const host = await setup();
        const third = { rules: { "default-width": { proportion: 1 / 3 } } };
        host.open(third);
        host.open(third);
        host.open(third);
        const w = host.open();

        // The seam nearest to W is the one between B and C, then the one between A and B.
        const seamX = (col) => (cellRect(0, 0, col, 12).w + cellRect(col, 0, 8, 12).x) / 2;
        host.action("stuff");
        assert.equal(middle(currentTarget(host).rect).x, seamX(16));
        assert.equal(targets(host).length, 2);

        host.action("focus-left");
        assert.equal(middle(currentTarget(host).rect).x, seamX(8));
        host.action("focus-right");
        assert.equal(middle(currentTarget(host).rect).x, seamX(16));
        assert.equal(host.focused(), w);

        host.action("toggle-full-width");
        assert.equal(hint(host), null);
        assert.equal(targets(host).length, 0);
    });

    test("stuffing between two windows of a group joins it", async () => {
        const host = await setup(half);
        const a = host.open();
        const b = host.open();
        host.groupWindows([a, b], a);
        const w = host.open({ rules: { "default-width": { fixed: 100 } } });
        host.action("stuff");
        host.action("stuff");
        assert.deepEqual(host.groupOf(a).windows, [a, w, b]);
        assert.equal(host.focused(), w);
    });

    test("targets sit on the edges windows share, also without gaps", async () => {
        const host = await setup({ ...half, config: { gaps: 0 } });
        const a = host.open();
        host.open();
        host.open({ rules: { "default-width": { fixed: 100 } } });
        host.action("stuff");
        const edge = host.rect(a).x + host.rect(a).w;
        assert.equal(middle(currentTarget(host).rect).x, edge);
    });

    test("there's nothing to stuff into without two windows touching", async () => {
        const host = await setup(half);
        host.open();
        host.action("stuff");
        assert.equal(hint(host), null);
    });
});

describe("layout API", () => {
    test("declares its actions, options and window rules", async () => {
        const host = await setup();
        assert.equal(host.meta.name, "bento");
        const names = host.meta.actions.map((action) => action.name);
        assert.equal(names[0], "focus-left");
        assert.deepEqual(names.slice(-4), ["split-h", "split-v", "swap", "stuff"]);
        // None of niri's names.
        assert.ok(!names.some((name) => name.includes("column")));
        const width = host.meta.actions.find((action) => action.name === "set-width");
        assert.deepEqual(
            width.args.map(({ name, type }) => [name, type]),
            [["change", "size"]],
        );
        const toWorkspace = host.meta.actions.find((action) => action.name === "move-to-workspace");
        assert.deepEqual(
            toWorkspace.args.map(({ name, type }) => [name, type]),
            [["workspace", "workspace"]],
        );
        assert.deepEqual(
            host.meta.options.map(({ name, default: value }) => [name, value]),
            [
                ["gravity", true],
                ["center-focused-window", "never"],
                ["always-center-single-window", false],
                ["default-width", "50%"],
                ["preset-widths", ["33.333%", "50%", "66.667%"]],
                ["preset-heights", ["33.333%", "50%", "66.667%"]],
                ["hint-color", "#7fc8ff80"],
            ],
        );
        assert.deepEqual(
            host.meta.windowRules.map(({ name, type }) => [name, type]),
            [
                ["default-width", "size"],
                ["default-height", "size"],
                ["open-full-width", "boolean"],
            ],
        );
    });

    test("unknown actions do nothing", async () => {
        const host = await setup();
        host.open();
        const before = cells(host);
        host.action("no-such-action");
        host.action("toString");
        assert.deepEqual(cells(host), before);
        assert.deepEqual(host.errors, []);
    });

    test("focus actions stop at the edges", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.action("focus-right");
        assert.equal(host.focused(), 2);
        host.action("focus-left");
        assert.equal(host.focused(), 1);
        host.action("focus-left");
        assert.equal(host.focused(), 1);
        host.action("focus-right-or-first");
        assert.equal(host.focused(), 2);
        host.action("focus-right-or-first");
        assert.equal(host.focused(), 1);
    });

    test("focus-index and move-to-index number the grid's columns", async () => {
        const host = await setUpTwoOverOne();
        host.activate(1);
        host.action("focus-index", 2);
        assert.equal(host.focused(), 2);

        // Indices past the end give the last column.
        host.activate(1);
        host.action("focus-index", 9);
        assert.equal(host.focused(), 2);

        // Window 2 moves to the first column, pushing window 1 along in its rows.
        host.activate(2);
        host.action("move-to-index", 1);
        assert.deepEqual(cells(host), ["2 × 6 at 2, 0", "2 × 6 at 0, 0", "2 × 6 at 0, 6"]);
    });

    test("focus-in-stack counts the stack", async () => {
        const host = await setUpOneAndStack();
        host.activate(2);
        host.action("focus-in-stack", 2);
        assert.equal(host.focused(), 3);
        host.action("focus-down-or-top");
        assert.equal(host.focused(), 2);
        host.action("focus-up-or-bottom");
        assert.equal(host.focused(), 3);
    });

    test("a new window opens next to the window it belongs to", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.open();
        host.open({ nextTo: 1 });
        assert.deepEqual(
            [1, 4, 2, 3].map((id) => host.rect(id).x),
            [0, 2, 4, 6].map((col) => cellRect(col, 0, 2, 12).x),
        );
    });

    test("windows opened without focus don't take it", async () => {
        const host = await setup();
        host.open();
        host.open({ activate: false });
        assert.equal(host.focused(), 1);
    });

    test("the camera only scrolls to keep the focused window in view", async () => {
        const host = await setup();
        for (let i = 0; i < 12; i++) host.open();

        // The last window is in view at the right edge.
        assert.deepEqual(cells(host, [12]), ["2 × 12 at 22, 0"]);
        const camera = host.camera().x;

        // Closing a window to the left doesn't move the view; the windows slide instead.
        host.close(11);
        assert.equal(host.camera().x, camera);
        assert.deepEqual(cells(host, [12]), ["2 × 12 at 20, 0"]);

        // Focusing a window out of view scrolls to it.
        host.activate(1);
        assert.deepEqual(cells(host, [1]), ["2 × 12 at 0, 0"]);
    });

    test("maximized windows fill the working area and push the others", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.activate(1);
        host.requestMode(1, "maximized");
        assert.equal(host.mode(1), "maximized");
        assert.deepEqual(tiles(host, [1]), ["1280 × 720 at x:0 y:0"]);
        assert.deepEqual(cells(host, [2]), ["2 × 12 at 24.7, 0"]);

        host.requestMode(1, "normal", { maximized: false });
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0", "2 × 12 at 2, 0"]);
    });

    test("mode requests make a window fullscreen and back", async () => {
        const host = await setup();
        host.open();
        host.requestMode(1, "fullscreen");
        assert.equal(host.mode(1), "fullscreen");
        assert.deepEqual(tiles(host), ["1280 × 720 at x:0 y:0"]);
        host.requestMode(1, "normal");
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0"]);
    });

    test("a group shows as a carousel, whose slits activate their windows", async () => {
        const host = await setup({ options: { "default-width": "50%" } });
        host.open();
        host.open();
        host.groupWindows([1, 2], 1);

        // The area is 616 px wide. Window 1, active, takes what window 2's slit of 40 px and half
        // of the 16 px gaps leave, and window 2, as wide, shows its middle in its slit.
        assert.deepEqual(host.rect(1), { x: 0, y: 16, w: 568, h: 688 });
        assert.deepEqual(host.rect(2), { x: 312, y: 16, w: 568, h: 688 });
        assert.equal(host.isVisible(2), true);
        assert.deepEqual(
            clips(host).map(({ rect, cornerRadius }) => [rect, cornerRadius]),
            [
                // Past the active window, so that its border and shadow aren't cut.
                [{ x: -64, y: -48, w: 696, h: 816 }, 0],
                [{ x: 576, y: 16, w: 40, h: 688 }, 12],
            ],
        );
        assert.deepEqual(
            [1, 2].map((id) => [parentOf(host, id), host.tree().node(`w${id}`).props.input]),
            [
                [clips(host)[0].id, "app"],
                [clips(host)[1].id, "engine"],
            ],
        );

        // The pointer over a slit widens it towards the active window.
        const [slit] = slits(host);
        host.pointer(slit.id, "pointerenter");
        assert.deepEqual(slits(host)[0].rect, { x: 560, y: 16, w: 56, h: 688 });
        host.pointer(slit.id, "pointerleave");
        assert.deepEqual(slits(host)[0].rect, { x: 576, y: 16, w: 40, h: 688 });

        // Clicking a slit's window activates it, and window 1 becomes a slit.
        host.pointer("w2", "click");
        assert.equal(host.focused(), 2);
        assert.equal(host.groupOf(1).active, 2);
        assert.deepEqual(host.rect(1), { x: -264, y: 16, w: 568, h: 688 });
        assert.deepEqual(host.rect(2), { x: 48, y: 16, w: 568, h: 688 });
        assert.deepEqual(slits(host).map((node) => node.rect), [{ x: 0, y: 16, w: 40, h: 688 }]);
        assert.equal(host.tree().node("w1").props.input, "engine");
        assert.equal(host.tree().node("w2").props.input, "app");

        // Only the active window shows while the group covers the view.
        host.requestMode(2, "fullscreen");
        assert.equal(slits(host).length, 0);
        assert.equal(host.isVisible(1), false);
        host.requestMode(2, "normal");
        assert.equal(slits(host).length, 1);

        // Moving a window reorders the group.
        host.action("move-left");
        assert.deepEqual(groupedAreas(host), [[2, 1]]);

        // Closing a window of the group keeps the area's cells, and puts window 1 back with the
        // other windows.
        host.close(2);
        assert.deepEqual(exposed(host).items.map((item) => item.area), [at(0, 0, 12, 12)]);
        assert.equal(host.focused(), 1);
        assert.equal(parentOf(host, 1), host.tree().rootId);
        assert.equal(host.tree().node("w1").props.input, "app");
        assert.equal(clips(host).length, 0);
    });

    test("a slit's window that bento moves away takes the pointer again", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.groupWindows([1, 2], 1);
        assert.equal(host.tree().node("w2").props.input, "engine");

        const to = host.workspace(1).id;
        host.move(2, to);
        assert.equal(host.tree(to).node("w2").props.input, "app");
        assert.equal(parentOf(host, 2, to), host.tree(to).rootId);
        assert.equal(clips(host).length, 0);
        assert.equal(parentOf(host, 1), host.tree().rootId);
    });

    test("a window's area fits its minimum width, and a group's its slits too", async () => {
        const host = await setup();
        host.open({ minSize: { w: 300, h: 0 } });
        host.open();

        // 100 px would be two cells, but window 1 needs 300 px: six cells of 37 px and their gaps,
        // and window 2 goes after it.
        assert.deepEqual(cells(host), ["6 × 12 at 0, 0", "2 × 12 at 6, 0"]);
        assert.deepEqual(host.windows.get(1).size, { w: 300, h: cellRect(0, 0, 6, 12).h });
        assert.deepEqual(host.errors, []);

        // As a group, the area also fits the 40 px slit beside window 1 and its 8 px gap: seven
        // cells, and window 2 moves out of the way.
        host.open();
        host.groupWindows([1, 3], 1);
        assert.ok(host.rect(1).w >= 300);
        assert.deepEqual(
            exposed(host).items.map((item) => [item.windows, item.area]),
            [
                [[1, 3], at(0, 0, 7, 12)],
                [[2], at(7, 0, 2, 12)],
            ],
        );
    });

    test("a window whose minimum width grows widens its area", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.setLimits(1, { minSize: { w: 300, h: 0 } });
        assert.ok(host.rect(1).w >= 300);
        assert.ok(host.rect(2).x >= host.rect(1).x + host.rect(1).w);
    });

    test("slits leave half of the gaps between them, and at most half of the area", async () => {
        const host = await setup();
        for (let i = 0; i < 3; i++) host.open();
        host.groupWindows([1, 2, 3], 2);
        assert.deepEqual(groupedAreas(host), [[2, 1, 3]]);

        // The area is 89 px wide: the slits of windows 1 and 3, 14 px each, and their gaps of
        // 8 px take half of it, and active window 2 the rest.
        assert.deepEqual(
            [2, 1, 3].map((id) => clips(host).find((node) => node.id === parentOf(host, id)).rect),
            [
                { x: -64, y: 16 - 64, w: 45 + 128, h: 688 + 128 },
                { x: 53, y: 16, w: 14, h: 688 },
                { x: 75, y: 16, w: 14, h: 688 },
            ],
        );
    });

    test("new windows are asked to open at the size of their cells", async () => {
        const host = await setup();
        const { w, h } = cellRect(0, 0, 2, 12);
        assert.deepEqual(host.newWindowSize(undefined, { rules: { "default-width": { fixed: 100 } } }), { w, h });
        assert.deepEqual(
            host.newWindowSize(undefined, {
                rules: {
                    "default-width": { proportion: 0.5 },
                    "default-height": { proportion: 0.5 },
                },
            }),
            { w: cellRect(0, 0, 12, 6).w, h: cellRect(0, 0, 12, 6).h },
        );
        assert.deepEqual(host.newWindowSize(undefined, { rules: { "default-width": null } }), { w: 0, h });
        assert.deepEqual(
            host.newWindowSize(undefined, {
                rules: { "default-width": { fixed: 100 } },
                minSize: { w: 300, h: 0 },
            }),
            { w: 300, h },
        );
    });

    test("windows keep their place and size through a reload", async () => {
        const host = await setUpTwoOverOne();
        host.actionOn(2, "set-width", set(50));
        host.activate(2);
        host.open();
        host.groupWindows([2, 4], 2);
        const before = {
            tiles: tiles(host),
            focused: host.focused(),
            ipc: ipc(host),
            grouped: groupedAreas(host),
        };
        assert.equal(before.grouped.length, 1);

        host.reload();
        assert.deepEqual(host.errors, []);
        assert.deepEqual(tiles(host), before.tiles);
        assert.equal(host.focused(), before.focused);
        assert.deepEqual(ipc(host), before.ipc);
        assert.deepEqual(groupedAreas(host), before.grouped);
        assert.equal(clips(host).length, before.grouped.flat().length);

        // Closing the window opened last still goes back to where it was opened from.
        host.close(4);
        assert.equal(host.focused(), 2);
    });

    test("windows from another layout snap to the cells under them", async () => {
        const host = await setup();
        const adopt = (rect, activate) => host.open({ reason: "adopt", activate, rect });
        // 12 cells by 12, and 6 cells by 5.
        adopt({ x: 10, y: 10, w: 600, h: 700 }, true);
        adopt({ x: 640, y: 20, w: 300, h: 300 }, false);
        assert.deepEqual(cells(host), ["12 × 12 at 0, 0", "6 × 5 at 12, 0"]);
    });

    test("turning gravity off and on", async () => {
        const host = await setUpHole();
        host.open();
        halfHeight(host, 3);
        host.action("move-down");

        // Without gravity, windows stay where they're moved.
        host.setOptions({ gravity: false });
        repeat(5, () => host.action("move-down"));
        assert.deepEqual(cells(host, [3]), ["2 × 6 at 4, 6"]);
        host.action("move-left");
        host.action("move-left");
        assert.deepEqual(cells(host, [3]), ["2 × 6 at 2, 6"]);

        // The leftmost window can go left of column 0 without gravity, but moving it alone left
        // just shifts everything, which isn't a change.
        host.activate(1);
        host.action("move-left");

        // Turning gravity back on slides windows left.
        host.activate(3);
        host.action("move-right");
        host.setOptions({ gravity: true });
        assert.deepEqual(cells(host, [3]), ["2 × 6 at 2, 6"]);
    });

    test("sizes stay proportional on a view with different cells", async () => {
        const host = await setup();
        host.open();
        host.actionOn(1, "set-width", set(50));
        host.open();
        host.actionOn(2, "set-width", set(50));
        host.activate(1);
        assert.deepEqual(cells(host), ["12 × 12 at 0, 0", "12 × 12 at 12, 0"]);

        // 36 columns across instead of 24: each window still covers half of them.
        host.resizeMonitor("DP-1", { size: { w: 3440, h: 1080 } });
        assert.equal(new CellMetrics({ w: 3440, h: 1080 }, R, 16).colsPerView, 36);
        assert.deepEqual(tiles(host), ["1696 × 1048 at x:16 y:16", "1696 × 1048 at x:1728 y:16"]);

        // Struts shrink the working area.
        host.config = { ...host.config, struts: { left: 0, right: 0, top: 100, bottom: 0 } };
        host.setOptions(host.options);
        assert.equal(host.screenRect(1).y, 116);
    });

    test("the highlight on an empty workspace is where the window will land", async () => {
        const host = await setup();
        const workspace = host.activeWorkspace();
        workspace.camera = { x: 5000, y: 0 };
        host.flush();
        const info = {
            id: 99,
            appId: null,
            title: null,
            workspace: workspace.id,
            size: { w: 0, h: 0 },
            minSize: { w: 0, h: 0 },
            maxSize: { w: 0, h: 0 },
            borders: { w: 0, h: 0 },
            decorations: { border: true, focusRing: true, shadow: true },
            mode: "normal",
            urgent: false,
            rules: {},
            data: null,
        };
        const target = dropTarget(
            host,
            99,
            { x: 5600, y: 300 },
            { x: 5500, y: 20, w: 99, h: 336 },
            info,
        );
        const cell = cellRect(0, 0, 2, 6);
        assert.deepEqual(
            { x: target.x - host.camera().x, y: target.y, w: target.w, h: target.h },
            { ...cell, x: cell.x + METRICS.gap },
        );
    });
});

describe("workspaces and monitors", () => {
    test("each workspace has a grid of its own", async () => {
        const host = await setup();
        const [w0, w1] = [host.workspace(0).id, host.workspace(1).id];
        host.open();
        host.showWorkspace(w1);
        host.open();
        assert.deepEqual(cells(host, [1, 2]), ["2 × 12 at 0, 0", "2 × 12 at 0, 0"]);
        assert.equal(host.windows.get(1).workspace, w0);
        assert.equal(host.windows.get(2).workspace, w1);
    });

    test("moving down from the bottom goes to the workspace below", async () => {
        const host = await setup();
        const [w0, w1] = [host.workspace(0).id, host.workspace(1).id];
        host.open();
        host.open();
        // The window is as tall as the grid, so it can't move down in it.
        host.action("move-down-or-to-workspace");
        assert.equal(host.windows.get(2).workspace, w1);
        assert.equal(host.activeWorkspace().id, w1);
        assert.equal(host.focused(), 2);
        assert.deepEqual(cells(host, [1, 2]), ["2 × 12 at 0, 0", "2 × 12 at 0, 0"]);

        host.action("move-up-or-to-workspace");
        assert.equal(host.windows.get(2).workspace, w0);
        assert.equal(host.activeWorkspace().id, w0);
        assert.deepEqual(cells(host, [1, 2]), ["2 × 12 at 0, 0", "2 × 12 at 2, 0"]);

        // With room below, it moves in the grid.
        halfHeight(host, 2);
        host.action("move-down-or-to-workspace");
        assert.equal(host.windows.get(2).workspace, w0);
        assert.deepEqual(cells(host, [2]), ["2 × 6 at 2, 1"]);
    });

    test("moving left from the start goes to the monitor to the left", async () => {
        const host = await setup({ monitors: [{ name: "DP-1" }, { name: "DP-2" }] });
        host.open({ workspace: host.activeWorkspace("DP-2").id });
        host.action("focus-right-or-monitor");
        assert.equal(host.focusedMonitor, "DP-2");
        host.action("move-left-or-to-monitor");
        assert.equal(host.windows.get(1).workspace, host.activeWorkspace("DP-1").id);
        assert.equal(host.focusedMonitor, "DP-1");
        assert.equal(host.focused(), 1);
    });

    test("move-to-workspace and move-to-monitor take the group along", async () => {
        const host = await setup({ monitors: [{ name: "DP-1" }, { name: "DP-2" }] });
        const w1 = host.workspace(1).id;
        host.open();
        host.open();
        host.open();
        host.groupWindows([1, 2], 1);
        host.activate(1);
        assert.deepEqual(groupedAreas(host), [[1, 2]]);

        host.action("move-to-workspace", w1);
        assert.equal(host.windows.get(1).workspace, w1);
        assert.equal(host.windows.get(2).workspace, w1);
        assert.equal(host.windows.get(3).workspace, host.workspace(0).id);
        assert.equal(host.activeWorkspace().id, w1);
        assert.deepEqual(groupedAreas(host), [[1, 2]]);
        assert.equal(host.focused(), 1);

        host.action("move-to-monitor", "DP-2");
        assert.equal(host.windows.get(1).workspace, host.activeWorkspace("DP-2").id);
        assert.equal(host.windows.get(2).workspace, host.activeWorkspace("DP-2").id);
        assert.equal(host.focusedMonitor, "DP-2");
        assert.deepEqual(groupedAreas(host), [[1, 2]]);
        assert.equal(slits(host).length, 1);
        // The workspaces it left keep nothing of its carousel.
        assert.equal(clips(host, w1).length, 0);
        assert.equal(clips(host, host.workspace(0, "DP-1").id).length, 0);
    });

    test("focus goes on to the monitor or workspace beside", async () => {
        const host = await setup({ monitors: [{ name: "DP-1" }, { name: "DP-2" }] });
        host.open();
        host.open({ workspace: host.activeWorkspace("DP-2").id });
        host.action("focus-right-or-monitor");
        assert.equal(host.focusedMonitor, "DP-2");
        assert.equal(host.focused(), 2);
        host.action("focus-left-or-monitor");
        assert.equal(host.focusedMonitor, "DP-1");
        assert.equal(host.focused(), 1);

        // Within a grid they focus windows.
        host.open();
        host.action("focus-left-or-monitor");
        assert.equal(host.focused(), 1);
        assert.equal(host.focusedMonitor, "DP-1");

        host.action("focus-down-or-workspace");
        assert.equal(host.activeWorkspace().id, host.workspace(1).id);
        host.action("focus-up-or-workspace");
        assert.equal(host.activeWorkspace().id, host.workspace(0).id);
        assert.equal(host.focused(), 1);
    });

    test("a window that bento moved gets an area of its own", async () => {
        const host = await setup();
        const w1 = host.workspace(1).id;
        host.open();
        host.open();
        host.move(2, w1);
        assert.equal(host.windows.get(2).workspace, w1);
        assert.deepEqual(cells(host, [1, 2]), ["2 × 12 at 0, 0", "2 × 12 at 0, 0"]);
        assert.equal(host.focused(), 1);
    });

    test("a monitor's cells come from its own size", async () => {
        const host = await setup({
            monitors: [{ name: "DP-1" }, { name: "DP-2", size: { w: 3440, h: 1080 } }],
        });
        host.open();
        host.open({ workspace: host.activeWorkspace("DP-2").id });
        host.action("focus-right-or-monitor");
        host.actionOn(2, "set-width", set(50));
        assert.equal(host.rect(2).w, 1696);
        host.actionOn(1, "set-width", set(50));
        assert.equal(host.rect(1).w, cellRect(0, 0, 12, 12).w);
    });
});

describe("edge double clicks", () => {
    test("left and right edges toggle the full width", async () => {
        const host = await setup();
        host.open();
        host.doubleClickEdge(1, { right: true });
        assert.deepEqual(cells(host), [`${METRICS.colsPerView} × 12 at 0, 0`]);
        host.doubleClickEdge(1, { left: true });
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0"]);
    });

    test("top and bottom edges toggle the full height", async () => {
        const host = await setup();
        host.open();
        halfHeight(host, 1);
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0"]);
        host.doubleClickEdge(1, { bottom: true });
        assert.deepEqual(cells(host), ["2 × 12 at 0, 0"]);
        host.doubleClickEdge(1, { top: true });
        assert.deepEqual(cells(host), ["2 × 6 at 0, 0"]);
    });
});

describe("what the engine exposes and draws", () => {
    test("the state has each area's windows and cells", async () => {
        const host = await setUpStackOfTwo();
        const state = exposed(host);
        assert.equal(state.active, 2);
        assert.equal(state.rows, 12);
        assert.equal(state.colsPerView, METRICS.colsPerView);
        assert.deepEqual(
            state.items.map(({ windows, active, area, grouped, mode }) => ({
                windows,
                active,
                area,
                grouped,
                mode,
            })),
            [
                { windows: [1], active: 1, area: at(0, 0, 2, 6), grouped: false, mode: "normal" },
                { windows: [2], active: 2, area: at(0, 6, 2, 6), grouped: false, mode: "normal" },
                { windows: [3], active: 3, area: at(2, 0, 2, 12), grouped: false, mode: "normal" },
            ],
        );

        // Those watching hear about changes, and only about changes.
        const changes = host.stateChanges;
        host.action("focus-right");
        assert.ok(host.stateChanges > changes);
        const again = host.stateChanges;
        host.action("focus-right");
        assert.equal(host.stateChanges, again);
    });

    test("the hint for a dragged window goes away when it leaves", async () => {
        const host = await setup();
        host.open();
        host.open();
        const info = lift(host, 2);
        const workspace = host.activeWorkspace().id;
        host.send({
            type: "dragenter",
            workspace,
            window: info,
            pointer: { x: 100, y: 100 },
            rect: { x: 90, y: 90, w: 100, h: 100 },
        });
        assert.notEqual(hint(host), null);
        host.send({ type: "dragleave", workspace });
        assert.equal(hint(host), null);
    });

    test("the hints follow their color option", async () => {
        const host = await setup({ options: { "hint-color": "#ff000080" } });
        host.open();
        host.action("swap");
        assert.ok(drawn(host).some((node) => node.background === "#ff000080"));
    });
});

// Groups are bento's, and the host keeps them like bento does: `groupWindows` is bento's own
// action that groups windows, and `groupOf` the group a window is in.
describe("groups", () => {
    test("grouping makes a bento group, and taking a window out of a group of two ends it", async () => {
        const host = await setUpStackOfTwo();
        assert.equal(host.groupOf(1), null);
        pickToGroup(host, [1, 2]);
        assert.deepEqual(host.groupOf(1).windows, [1, 2]);
        assert.equal(host.groupOf(1).active, 2);
        host.action("focus-left");
        assert.equal(host.groupOf(1).active, 1);
        pickToGroup(host, [2]);
        assert.equal(host.groupOf(1), null);
        assert.equal(host.groupOf(2), null);
    });

    test("a group made elsewhere shows as a carousel in the area of its active window", async () => {
        const host = await setup();
        host.open();
        host.open();
        host.open();
        host.groupWindows([1, 3], 3);
        assert.deepEqual(groupedAreas(host), [[3, 1]]);
        assert.equal(host.isVisible(3), true);
        assert.equal(host.isVisible(1), true);
        assert.equal(slits(host).length, 1);
        assert.deepEqual(
            exposed(host).items.map((item) => [item.windows, item.area]),
            [
                [[2], at(0, 0, 2, 12)],
                [[3, 1], at(2, 0, 2, 12)],
            ],
        );
    });

    test("a window that leaves its group's area leaves the group", async () => {
        const host = await setUpStackOfTwo();
        host.groupWindows([1, 2], 1);
        host.activate(1);
        host.action("unstack");
        assert.equal(host.groupOf(1), null);
        assert.equal(host.groupOf(2), null);
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
        // moving right.
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

    test("metrics in twelfths", () => {
        const m = new CellMetrics({ w: 1920, h: 1080 }, R, 16);
        // 22 columns would be closer to square, but 24 is the nearest multiple of 12.
        assert.equal(m.colsPerView, 24);

        // The cells exactly fill the view.
        const full = m.rect(A(0, 0, m.colsPerView, R));
        assert.ok(Math.abs(full.x + full.w + m.gap - 1920) < 1e-6);
        assert.ok(Math.abs(full.y + full.h + m.gap - 1080) < 1e-6);

        assert.equal(m.rowsForProportion(0.5), 6);
        assert.equal(m.rowsForProportion(1 / 3), 4);
        assert.equal(m.rowsForProportion(0.25), 3);
        assert.equal(m.rowsForProportion(0.33333), 4);

        // Halves, thirds and quarters of the width are whole numbers of columns.
        assert.equal(m.colsForProportion(0.5), 12);
        assert.equal(m.colsForProportion(0.33333), 8);
        assert.equal(m.colsForProportion(0.66667), 16);
        assert.equal(m.colsForProportion(0.25), 6);
        assert.equal(m.colsForProportion(0.75), 18);

        // A wider view gets the next multiple of 12.
        assert.equal(new CellMetrics({ w: 3440, h: 1080 }, R, 16).colsPerView, 36);
        // A narrow one still gets 12.
        assert.equal(new CellMetrics({ w: 400, h: 1080 }, R, 16).colsPerView, 12);

        // The test output.
        assert.equal(METRICS.colsPerView, 24);
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
