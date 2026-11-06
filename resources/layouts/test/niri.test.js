// Tests for the niri layout. Expected rects follow niri's own math for a 1280 × 720 output with
// 16 px gaps: a half-width column is (1280 - 16) * 0.5 - 16 = 616 px wide and 720 - 32 = 688 px
// tall, and the first column shows at x 16 with the camera at -16.

import { test, describe } from "node:test";
import assert from "node:assert/strict";

import { TestHost } from "./host.js";

const load = (opts) => TestHost.load("../niri.js", opts);
const rect = (x, y, w, h) => ({ x, y, w, h });
const px = (value) => ({ op: "set", value, unit: "px" });
const percent = (value) => ({ op: "set", value, unit: "%" });
const adjust = (value, unit) => ({ op: "adjust", value, unit });

/** Opens windows and returns their ids. */
function openMany(host, count, opts) {
    return Array.from({ length: count }, () => host.open(opts));
}

/** Opens two windows stacked in one column, the second one focused. */
function openStacked(host) {
    const [a, b] = openMany(host, 2);
    host.action("consume-or-expel-window-left");
    return [a, b];
}

/** The columns of a workspace as lists of window ids, from the state the engine exposes. */
function columnsOf(host, workspace = host.activeWorkspace().id) {
    const state = host.exposedState().workspaces.find((ws) => ws.id === workspace);
    return state.columns.map((col) => col.windows);
}

/** The 1-based column and tile of a window, from the state the engine exposes. */
function position(host, id) {
    const columns = host.exposedState().workspaces.flatMap((ws) => ws.columns.map((col) => col.windows));
    const col = columns.findIndex((windows) => windows.includes(id));
    return [col + 1, columns[col].indexOf(id) + 1];
}

/** Window rules that set a width, and maybe a height, like a matching `window-rule` does. */
function wide(width, height) {
    return {
        rules: {
            "default-column-width": width,
            ...(height === undefined ? {} : { "default-window-height": height }),
        },
    };
}

/** What the engine draws on a workspace: the boxes, in drawing order. */
function drawn(host, workspace = host.activeWorkspace().id) {
    const tree = host.tree(workspace);
    return tree
        .order()
        .filter((id) => tree.node(id).kind === "box")
        .map((id) => ({ id, ...tree.node(id).props }));
}

/** The tab indicator's tabs, which are the interactive boxes. */
const tabsOf = (host, workspace) => drawn(host, workspace).filter((node) => node.interactive);

/** The insert hint, if one is shown. */
const hintOf = (host, workspace) =>
    drawn(host, workspace).find((node) => node.background === "#7fc8ff80" && !node.interactive) ?? null;

/** Starts dragging a window: it leaves its workspace. Returns what bento tells about it. */
function startDrag(host, id) {
    const win = host.windows.get(id);
    const info = host.windowInfo(win);
    host.close(id, "drag");
    host.windows.set(id, win);
    return info;
}

/** Moves the pointer of a dragged window over a workspace. */
function dragOver(host, info, pointer, workspace = host.activeWorkspace().id, first = false) {
    host.send({
        type: first ? "dragenter" : "dragmove",
        workspace,
        window: info,
        pointer,
        rect: { x: pointer.x - 10, y: pointer.y - 10, w: 20, h: 20 },
    });
}

/** The rect of the insert hint while dragging `id` with the pointer at `pointer`. */
function hintAt(host, id, pointer) {
    const info = startDrag(host, id);
    dragOver(host, info, pointer, undefined, true);
    const hint = hintOf(host)?.rect ?? null;
    host.send({ type: "dragleave", workspace: host.activeWorkspace().id });
    return hint;
}

/** Drops a dragged window with the pointer at `pointer`. */
function drop(host, id, pointer, workspace = host.activeWorkspace().id) {
    const win = host.windows.get(id);
    win.workspace = workspace;
    host.trees.get(workspace).add(`w${id}`);
    host.told.windows.set(id, host.windowInfo(win));
    host.send({
        type: "windowadd",
        window: host.windowInfo(win),
        how: {
            reason: "drop",
            activate: true,
            rect: { x: pointer.x - 10, y: pointer.y - 10, w: 20, h: 20 },
            pointer,
            nextTo: null,
        },
    });
}

describe("opening windows", () => {
    test("the first window fills the height with the default width", async () => {
        const host = await load();
        const a = host.open();
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.camera(), { x: -16, y: 0, zoom: 1 });
        assert.deepEqual(host.screenRect(a), rect(16, 16, 616, 688));
        assert.equal(host.focused(), a);
        assert.deepEqual(host.errors, []);
    });

    test("new windows open right of the focused column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        // Both fit, so the view doesn't move.
        assert.equal(host.camera().x, -16);

        // The third doesn't fit: the view scrolls the least it can to show it.
        const c = host.open();
        assert.deepEqual(host.rect(c), rect(1264, 16, 616, 688));
        assert.equal(host.camera().x, 616);
        assert.deepEqual(host.screenRect(b), rect(16, 16, 616, 688));
        assert.deepEqual(host.screenRect(c), rect(648, 16, 616, 688));
        assert.equal(host.focused(), c);

        // Opening from the first column puts the window right after it.
        host.action("focus-column-first");
        const d = host.open();
        assert.deepEqual(columnsOf(host), [[a], [d], [b], [c]]);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(d), rect(632, 16, 616, 688));
    });

    test("windows that don't activate don't take focus or move the view", async () => {
        const host = await load();
        const a = host.open();
        const b = host.open({ activate: false });
        assert.equal(host.focused(), a);
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.equal(host.camera().x, -16);
    });

    test("windows open next to the window they belong to", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        const d = host.open({ nextTo: a });
        assert.deepEqual(columnsOf(host), [[a], [d], [b], [c]]);
        assert.equal(host.focused(), d);
    });

    test("default width and height from the window rules", async () => {
        const host = await load();
        const a = host.open(wide({ fixed: 500 }, { proportion: 0.5 }));
        // 500 px is for the window; the tile has the borders too.
        const b = host.open({ ...wide({ fixed: 500 }), borders: { w: 8, h: 8 } });
        const c = host.open(wide({ proportion: 1 / 3 }));
        assert.deepEqual(host.rect(a), rect(0, 16, 500, 336));
        assert.deepEqual(host.rect(b), rect(516, 16, 508, 688));
        assert.deepEqual(host.rect(c), rect(1040, 16, 405, 688));
    });

    test("struts and panels shrink the working area", async () => {
        const host = await load({
            monitors: [{ name: "DP-1", workingArea: { x: 0, y: 30, w: 1280, h: 690 } }],
            config: { struts: { left: 50, right: 0, top: 0, bottom: 0 } },
        });
        const a = host.open();
        // (1230 - 16) * 0.5 - 16 wide, 690 - 32 tall, below the panel.
        assert.deepEqual(host.rect(a), rect(0, 46, 591, 658));
        assert.deepEqual(host.screenRect(a), rect(66, 46, 591, 658));
    });

    test("a window that opens without focus on an empty workspace stays unfocused", async () => {
        const host = await load();
        const a = host.open({ activate: false });
        assert.equal(host.focused(), null);
        assert.deepEqual(host.screenRect(a), rect(16, 16, 616, 688));
    });

    test("windows that stop floating keep their width", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.close(a, "float");
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        const c = host.open({ reason: "tile", rect: rect(100, 100, 500, 400) });
        assert.deepEqual(host.rect(c), rect(632, 16, 500, 688));
        assert.equal(host.focused(), c);
    });

    test("min sizes widen the column", async () => {
        const host = await load();
        const a = host.open({ minSize: { w: 700, h: 0 } });
        const b = host.open();
        assert.deepEqual(host.rect(a), rect(0, 16, 700, 688));
        assert.deepEqual(host.rect(b), rect(716, 16, 616, 688));
    });
});

describe("closing windows", () => {
    test("closing a window that just opened goes back to the previous one", async () => {
        const host = await load();
        const [, b, c] = openMany(host, 3);
        assert.equal(host.camera().x, 616);
        host.close(c);
        assert.equal(host.focused(), b);
        // The view goes back to where it was before the window opened.
        assert.equal(host.camera().x, -16);
    });

    test("after focus moves, closing the last column keeps the view", async () => {
        const host = await load();
        const [, b, c] = openMany(host, 3);
        host.action("focus-column-left");
        host.action("focus-column-right");
        host.close(c);
        assert.equal(host.focused(), b);
        assert.equal(host.camera().x, 616);
        assert.deepEqual(host.screenRect(b), rect(16, 16, 616, 688));
    });

    test("the next column takes the place of a closed one", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-left");
        host.close(b);
        assert.equal(host.focused(), c);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(c), rect(632, 16, 616, 688));
        assert.deepEqual(host.screenRect(c), rect(16, 16, 616, 688));
    });

    test("closing a window in a column gives its height to the others", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(b), rect(0, 368, 616, 336));
        host.close(b);
        assert.equal(host.focused(), a);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
    });

    test("closing a window above the focused one keeps focus", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-first");
        host.action("consume-window-into-column");
        host.action("consume-window-into-column");
        host.action("focus-window-down");
        host.close(a);
        assert.equal(host.focused(), b);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(c), rect(0, 368, 616, 336));
    });

    test("the first window on an emptied workspace resets the view", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        for (const id of [a, b, c]) host.close(id);
        assert.equal(host.focused(), null);
        const d = host.open();
        assert.deepEqual(host.rect(d), rect(0, 16, 616, 688));
        assert.equal(host.camera().x, -16);
    });
});

describe("focus actions", () => {
    test("focusing columns", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-right");
        host.action("focus-column-left");
        assert.equal(host.focused(), b);
        // b is fully visible, so the view stays.
        assert.equal(host.camera().x, 616);
        host.action("focus-column-left");
        assert.equal(host.focused(), a);
        assert.equal(host.camera().x, -16);
        host.action("focus-column-left");
        host.action("focus-column-last");
        assert.equal(host.focused(), c);
        host.action("focus-column-first");
        assert.equal(host.focused(), a);
        host.action("focus-column-left-or-last");
        assert.equal(host.focused(), c);
        host.action("focus-column-right-or-first");
        assert.equal(host.focused(), a);
        host.action("focus-column", 2);
        assert.equal(host.focused(), b);
        host.action("focus-column", 10);
        assert.equal(host.focused(), c);
    });

    test("focusing windows in a column", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-first");
        host.action("consume-window-into-column");
        host.action("consume-window-into-column");
        assert.equal(host.focused(), a);
        host.action("focus-window-up");
        host.action("focus-window-down");
        assert.equal(host.focused(), b);
        host.action("focus-window-bottom");
        assert.equal(host.focused(), c);
        host.action("focus-window-down");
        host.action("focus-window-down-or-top");
        assert.equal(host.focused(), a);
        host.action("focus-window-up-or-bottom");
        assert.equal(host.focused(), c);
        host.action("focus-window-top");
        assert.equal(host.focused(), a);
        host.action("focus-window-in-column", 2);
        assert.equal(host.focused(), b);
        host.action("focus-window-in-column", 9);
        assert.equal(host.focused(), c);
    });

    test("focusing windows or columns", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        const c = host.open();
        host.action("focus-column-left");
        assert.equal(host.focused(), b);
        host.action("focus-window-down-or-column-right");
        assert.equal(host.focused(), c);
        host.action("focus-window-up-or-column-left");
        assert.equal(host.focused(), b);
        host.action("focus-window-up-or-column-right");
        assert.equal(host.focused(), a);
        host.action("focus-window-down-or-column-left");
        assert.equal(host.focused(), b);
        host.action("focus-window-down-or-column-left");
        assert.equal(host.focused(), b);
    });

    test("activating a window focuses its column and tile", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        const c = host.open();
        host.activate(a);
        assert.equal(host.focused(), a);
        assert.deepEqual(position(host, c), [2, 1]);
        host.activate(c);
        assert.equal(host.focused(), c);
        host.activate(b);
        assert.equal(host.focused(), b);
    });
});

describe("move actions", () => {
    test("moving columns", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        const order = () => [a, b, c].sort((x, y) => host.rect(x).x - host.rect(y).x);
        host.action("move-column-right");
        host.action("move-column-left");
        assert.deepEqual(order(), [a, c, b]);
        assert.equal(host.focused(), c);
        host.action("move-column-to-first");
        assert.deepEqual(order(), [c, a, b]);
        assert.equal(host.camera().x, -16);
        host.action("move-column-left");
        host.action("move-column-to-last");
        assert.deepEqual(order(), [a, b, c]);
        host.action("move-column-to-index", 2);
        assert.deepEqual(order(), [a, c, b]);
        assert.equal(host.focused(), c);
        assert.deepEqual(position(host, c), [2, 1]);
    });

    test("moving windows in a column", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.action("move-window-down");
        host.action("move-window-up");
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(a), rect(0, 368, 616, 336));
        assert.equal(host.focused(), b);
        host.action("move-window-up");
        host.action("move-window-down");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 336));
    });
});

describe("consuming and expelling", () => {
    test("consume or expel left and right", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        assert.deepEqual(position(host, b), [1, 2]);
        assert.equal(host.focused(), b);
        assert.equal(host.camera().x, -16);

        // Expelling right puts the window in a new column.
        host.action("consume-or-expel-window-right");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.equal(host.focused(), b);

        // A single window can't go further right.
        host.action("consume-or-expel-window-right");
        host.action("focus-column-left");
        host.action("consume-or-expel-window-right");
        assert.deepEqual(position(host, a), [1, 2]);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(a), rect(0, 368, 616, 336));
        assert.equal(host.focused(), a);

        // Expelling left puts the window in a new column on the left.
        host.action("consume-or-expel-window-left");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.equal(host.focused(), a);
        host.action("consume-or-expel-window-left");
    });

    test("consume or expel a window by id", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.actionOn(a, "consume-or-expel-window-right");
        assert.deepEqual(position(host, a), [1, 2]);
        assert.deepEqual(position(host, c), [2, 1]);
        // The focused window stays focused.
        assert.equal(host.focused(), c);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 336));
    });

    test("consume into and expel from the focused column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.action("focus-column-left");
        host.action("consume-window-into-column");
        assert.equal(host.focused(), a);
        assert.deepEqual(host.rect(b), rect(0, 368, 616, 336));
        host.action("consume-window-into-column");
        host.action("expel-window-from-column");
        assert.equal(host.focused(), a);
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        host.action("expel-window-from-column");
    });

    test("swapping windows between columns", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-left");
        host.action("consume-window-into-column");
        // Columns: [a], [b, c] with b focused.
        host.action("swap-window-left");
        assert.equal(host.focused(), b);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(a), rect(632, 16, 616, 336));
        assert.deepEqual(host.rect(c), rect(632, 368, 616, 336));
        host.action("swap-window-left");

        host.action("swap-window-right");
        assert.equal(host.focused(), b);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 336));
    });

    test("swapping single windows moves the column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.action("swap-window-left");
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(a), rect(632, 16, 616, 688));
        assert.equal(host.focused(), b);
    });
});

describe("column widths", () => {
    test("set-column-width", async () => {
        const host = await load();
        const a = host.open();
        host.action("set-column-width", px(800));
        assert.deepEqual(host.rect(a), rect(0, 16, 800, 688));
        host.action("set-column-width", percent(25));
        assert.equal(host.rect(a).w, 300);
        host.action("set-column-width", adjust(10, "%"));
        // (1264 * 0.35) - 16, rounded down.
        assert.equal(host.rect(a).w, 426);
        host.action("set-column-width", adjust(-26, "px"));
        assert.equal(host.rect(a).w, 400);
        host.action("set-column-width", adjust(10, "%"));
        // A fixed width goes back to a proportion: (400 + 16) / 1264 + 0.1.
        assert.equal(host.rect(a).w, 526);
    });

    test("set-window-width on a window gives it that width", async () => {
        const host = await load();
        const a = host.open({ borders: { w: 4, h: 4 } });
        const b = host.open();
        host.actionOn(a, "set-window-width", px(500));
        assert.deepEqual(host.rect(a), rect(0, 16, 504, 688));
        assert.deepEqual(host.rect(b), rect(520, 16, 616, 688));
        assert.equal(host.focused(), b);
    });

    test("switching preset widths", async () => {
        const host = await load();
        const a = host.open();
        // The default width matches the 1/2 preset, so the next one is 2/3.
        host.action("switch-preset-column-width");
        assert.equal(host.rect(a).w, 826);
        host.action("switch-preset-column-width");
        assert.equal(host.rect(a).w, 405);
        host.action("switch-preset-column-width-back");
        assert.equal(host.rect(a).w, 826);
        host.action("switch-preset-column-width", false);
        assert.equal(host.rect(a).w, 616);

        // From a width that isn't a preset, go to the next larger one.
        host.action("set-column-width", px(700));
        host.action("switch-preset-window-width");
        assert.equal(host.rect(a).w, 826);
        host.action("set-column-width", px(700));
        host.action("switch-preset-window-width-back");
        assert.equal(host.rect(a).w, 616);
    });

    test("maximize-column toggles full width", async () => {
        const host = await load();
        const a = host.open();
        host.action("maximize-column");
        assert.deepEqual(host.rect(a), rect(0, 16, 1248, 688));
        assert.equal(host.camera().x, -16);
        host.action("maximize-column");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
    });

    test("expand-column-to-available-width", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2, wide({ proportion: 1 / 3 }));
        assert.deepEqual(host.rect(b), rect(421, 16, 405, 688));
        host.action("expand-column-to-available-width");
        assert.deepEqual(host.rect(a), rect(0, 16, 405, 688));
        assert.deepEqual(host.rect(b), rect(421, 16, 827, 688));
        assert.equal(host.camera().x, -16);
        // Nothing left to expand into.
        host.action("expand-column-to-available-width");

        // A lone column goes full width.
        host.close(a);
        host.action("expand-column-to-available-width");
        assert.deepEqual(host.rect(b), rect(0, 16, 1248, 688));
    });
});

describe("window heights", () => {
    test("set-window-height and reset-window-height", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.action("set-window-height", px(200));
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 472));
        assert.deepEqual(host.rect(b), rect(0, 504, 616, 200));
        host.actionOn(a, "set-window-height", px(300));
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 300));
        assert.deepEqual(host.rect(b), rect(0, 332, 616, 372));
        host.actionOn(a, "set-window-height", adjust(-50, "px"));
        assert.equal(host.rect(a).h, 250);
        // Other windows keep their min height.
        host.actionOn(a, "set-window-height", px(5000));
        assert.equal(host.rect(a).h, 671);
        assert.equal(host.rect(b).h, 1);
        // Resetting gives a an auto height again; b keeps the weight it got when a's height
        // was first set, from the heights then.
        host.actionOn(a, "reset-window-height");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 472));
        assert.deepEqual(host.rect(b), rect(0, 504, 616, 200));
        host.actionOn(b, "reset-window-height");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(b), rect(0, 368, 616, 336));
    });

    test("a single window can be shorter than the column", async () => {
        const host = await load();
        const a = host.open();
        host.action("set-window-height", percent(50));
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 336));
    });

    test("switching preset heights", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        // Presets resolve to 219, 336 and 453 px; the next one up from 336 is 453.
        host.action("switch-preset-window-height");
        assert.deepEqual(host.rect(b), rect(0, 251, 616, 453));
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 219));
        host.action("switch-preset-window-height");
        assert.equal(host.rect(b).h, 219);
        assert.equal(host.rect(a).h, 453);
        host.action("switch-preset-window-height-back");
        assert.equal(host.rect(b).h, 453);
        host.actionOn(a, "switch-preset-window-height");
        // Only one window has a non-auto height: a goes up from 219 to the next preset, and b
        // goes back to auto.
        assert.equal(host.rect(a).h, 336);
        assert.equal(host.rect(b).h, 336);
    });

    test("min heights are kept", async () => {
        const host = await load();
        const a = host.open();
        const b = host.open({ minSize: { w: 0, h: 500 } });
        host.action("consume-or-expel-window-left");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 172));
        assert.deepEqual(host.rect(b), rect(0, 204, 616, 500));
    });
});

describe("centering", () => {
    test("center-column and center-window", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.action("center-column");
        assert.deepEqual(host.screenRect(b), rect(332, 16, 616, 688));
        // Only the focused column can be centered.
        host.actionOn(a, "center-window");
        host.action("focus-column-left");
        host.actionOn(a, "center-window");
        assert.deepEqual(host.screenRect(a), rect(332, 16, 616, 688));
    });

    test("center-visible-columns", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2, wide({ proportion: 1 / 3 }));
        host.action("center-visible-columns");
        // 405 + 16 + 405 px of columns, centered in 1280 px.
        assert.deepEqual(host.screenRect(a), rect(227, 16, 405, 688));
        assert.deepEqual(host.screenRect(b), rect(648, 16, 405, 688));
    });

    test("center-focused-column always", async () => {
        const host = await load({ options: { "center-focused-column": "always" } });
        const a = host.open();
        assert.deepEqual(host.screenRect(a), rect(332, 16, 616, 688));
        const b = host.open();
        assert.deepEqual(host.screenRect(b), rect(332, 16, 616, 688));
        host.action("focus-column-left");
        assert.deepEqual(host.screenRect(a), rect(332, 16, 616, 688));
    });

    test("center-focused-column on-overflow", async () => {
        const host = await load({ options: { "center-focused-column": "on-overflow" } });
        const [, b] = openMany(host, 2);
        // Two half columns fit together.
        assert.deepEqual(host.screenRect(b), rect(648, 16, 616, 688));
        // A wide one doesn't fit with the previous one, so it's centered.
        const c = host.open(wide({ proportion: 2 / 3 }));
        assert.deepEqual(host.screenRect(c), rect(227, 16, 826, 688));
    });

    test("always-center-single-column", async () => {
        const host = await load({ options: { "always-center-single-column": true } });
        const a = host.open();
        assert.deepEqual(host.screenRect(a), rect(332, 16, 616, 688));
        const b = host.open();
        assert.deepEqual(host.screenRect(b), rect(648, 16, 616, 688));
        host.close(b);
        assert.deepEqual(host.screenRect(a), rect(332, 16, 616, 688));
    });
});

describe("tabbed columns", () => {
    test("tabs share the column and only the active one shows", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.action("toggle-column-tabbed-display");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.equal(host.isVisible(a), false);
        assert.equal(host.isVisible(b), true);

        host.action("focus-window-up");
        assert.equal(host.focused(), a);
        assert.equal(host.isVisible(a), true);
        assert.equal(host.isVisible(b), false);

        // New tabs go in the background.
        const c = host.open();
        host.action("consume-or-expel-window-left");
        assert.equal(host.focused(), c);
        assert.deepEqual(host.rect(c), rect(0, 16, 616, 688));
        assert.deepEqual(
            [a, b, c].map((id) => host.isVisible(id)),
            [false, false, true],
        );

        host.action("set-column-display", "tabbed");
        host.action("set-column-display", "normal");
        assert.deepEqual(
            [a, b, c].map((id) => host.isVisible(id)),
            [true, true, true],
        );
        // 656 px for three: rounded as they're handed out.
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 219));
        assert.deepEqual(host.rect(b), rect(0, 251, 616, 219));
        assert.deepEqual(host.rect(c), rect(0, 486, 616, 218));
    });

    test("tabbed columns draw a tab indicator", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        assert.equal(tabsOf(host).length, 0);
        host.action("toggle-column-tabbed-display");
        // The tabs are along the left of the column, half its height, and the shown window's is
        // in the active color.
        const tabs = tabsOf(host);
        assert.deepEqual(
            tabs.map((tab) => [tab.rect, tab.background]),
            [
                [rect(-9, 188, 4, 172), "#505050"],
                [rect(-9, 360, 4, 172), "#7fc8ff"],
            ],
        );

        // Clicking a tab switches to it.
        host.pointer(tabs[0].id, "click");
        assert.equal(host.focused(), a);
        assert.equal(host.isVisible(a), true);
        assert.equal(host.isVisible(b), false);
        assert.equal(tabsOf(host)[0].background, "#7fc8ff");

        // Fullscreen hides the indicator.
        host.requestMode(a, "fullscreen");
        assert.equal(tabsOf(host).length, 0);
        host.requestMode(a, "normal");
        assert.equal(tabsOf(host).length, 2);

        host.action("set-column-display", "normal");
        assert.equal(tabsOf(host).length, 0);
    });

    test("the tab indicator follows its options", async () => {
        const host = await load({
            options: {
                "tab-indicator-position": "top",
                "tab-indicator-width": 6,
                "tab-indicator-gap": 2,
                "tab-indicator-length": 1,
                "tab-indicator-active-color": "#112233",
                "tab-indicator-inactive-color": "#445566",
                "tab-indicator-urgent-color": "#778899",
                "tab-indicator-corner-radius": 3,
            },
        });
        const [a] = openStacked(host);
        host.action("toggle-column-tabbed-display");
        host.windows.get(a).urgent = true;
        host.flush();
        // Above the column, along its whole width, with the urgent tab in its own color.
        assert.deepEqual(
            tabsOf(host).map((tab) => [tab.rect, tab.background, tab.cornerRadius]),
            [
                [rect(0, 8, 308, 6), "#778899", 3],
                [rect(308, 8, 308, 6), "#112233", 3],
            ],
        );
        const off = await load({ options: { "tab-indicator": false } });
        openStacked(off);
        off.action("toggle-column-tabbed-display");
        assert.equal(tabsOf(off).length, 0);

        const single = await load({ options: { "tab-indicator-hide-when-single-tab": true } });
        single.open();
        single.action("toggle-column-tabbed-display");
        assert.equal(tabsOf(single).length, 0);
    });

    test("the tabs of the indicator can have space between them", async () => {
        const host = await load({ options: { "tab-indicator-gaps-between-tabs": 4 } });
        openStacked(host);
        host.action("toggle-column-tabbed-display");
        // The same 344 px of indicator, with 4 px between the two tabs.
        assert.deepEqual(
            tabsOf(host).map((tab) => tab.rect),
            [rect(-9, 188, 4, 170), rect(-9, 362, 4, 170)],
        );

        const one = await load({ options: { "tab-indicator-gaps-between-tabs": 4 } });
        one.open();
        one.action("toggle-column-tabbed-display");
        assert.deepEqual(tabsOf(one).map((tab) => tab.rect), [rect(-9, 188, 4, 344)]);
    });

    test("tabs can be painted with gradients, which win over colors", async () => {
        const active = { from: "#112233", to: "#445566", angle: 90, relativeTo: "view", in: "oklab" };
        const urgent = { from: "#aa0000", to: "#ff0000" };
        const host = await load({
            options: { "tab-indicator-active-gradient": active, "tab-indicator-urgent-gradient": urgent },
        });
        const [a] = openStacked(host);
        host.action("toggle-column-tabbed-display");
        assert.deepEqual(
            tabsOf(host).map((tab) => tab.background),
            ["#505050", active],
        );

        host.windows.get(a).urgent = true;
        host.flush();
        assert.deepEqual(
            tabsOf(host).map((tab) => tab.background),
            [urgent, active],
        );

        // A gradient that's taken away leaves the color.
        host.setOptions({ "tab-indicator-active-gradient": null });
        assert.equal(tabsOf(host)[1].background, "#7fc8ff");
    });

    test("a window's rules paint its tab, over the options", async () => {
        const gradient = { from: "#000000", to: "#ffffff", angle: 0 };
        const option = { from: "#111111", to: "#222222" };
        const host = await load({ options: { "tab-indicator-active-gradient": option } });
        const a = host.open({
            rules: {
                "tab-indicator-inactive-color": "#010203",
                "tab-indicator-active-gradient": gradient,
            },
        });
        // A rule's color is before the option's gradient, like niri's.
        const b = host.open({ rules: { "tab-indicator-active-color": "#abcdef" } });
        host.action("consume-or-expel-window-left");
        host.action("toggle-column-tabbed-display");
        assert.equal(host.focused(), b);
        assert.deepEqual(
            tabsOf(host).map((tab) => tab.background),
            ["#010203", "#abcdef"],
        );

        // The other way around, with each window's active tab.
        host.pointer(tabsOf(host)[0].id, "click");
        assert.equal(host.focused(), a);
        assert.deepEqual(
            tabsOf(host).map((tab) => tab.background),
            [gradient, "#505050"],
        );
    });

    test("a tab indicator within the column takes room from the tabs", async () => {
        const host = await load({
            options: { "tab-indicator-gap": 6, "tab-indicator-place-within-column": true },
        });
        const [a, b] = openStacked(host);
        host.action("toggle-column-tabbed-display");
        // 10 px for the indicator: the tabs are 10 px narrower and start 10 px to the right, and
        // the column keeps its width.
        assert.deepEqual(host.rect(b), rect(10, 16, 606, 688));
        const c = host.open();
        assert.deepEqual(host.rect(c), rect(632, 16, 616, 688));

        // Dropping into the column highlights the tabs, not the indicator.
        assert.deepEqual(hintAt(host, c, { x: 300, y: 700 }), rect(10, 554, 606, 150));
        assert.deepEqual(host.rect(a), rect(10, 16, 606, 688));
    });

    test("new columns can be tabbed by default", async () => {
        const host = await load({ options: { "default-column-display": "tabbed" } });
        const [a, b] = openStacked(host);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.equal(host.isVisible(a), false);
        assert.equal(host.isVisible(b), true);
    });
});

describe("fullscreen and maximized", () => {
    test("a fullscreen column covers the view, and the view comes back after", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.requestMode(b, "fullscreen");
        assert.equal(host.mode(b), "fullscreen");
        assert.deepEqual(host.rect(b), rect(632, 0, 1280, 720));
        assert.deepEqual(host.screenRect(b), rect(0, 0, 1280, 720));
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));

        host.requestMode(b, "normal");
        assert.equal(host.mode(b), "normal");
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.equal(host.camera().x, -16);
    });

    test("fullscreen takes a window out of its column", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.requestMode(a, "fullscreen");
        assert.deepEqual(position(host, a), [2, 1]);
        assert.deepEqual(host.rect(a), rect(632, 0, 1280, 720));
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.equal(host.focused(), b);
    });

    test("tabbed columns go fullscreen together", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.action("toggle-column-tabbed-display");
        host.requestMode(b, "fullscreen");
        assert.deepEqual(
            [a, b].map((id) => host.mode(id)),
            ["fullscreen", "fullscreen"],
        );
        assert.deepEqual(host.rect(b), rect(0, 0, 1280, 720));
        assert.equal(host.camera().x, 0);
        // Leaving tabbed display leaves fullscreen.
        host.action("set-column-display", "normal");
        assert.deepEqual(
            [a, b].map((id) => host.mode(id)),
            ["normal", "normal"],
        );
        assert.equal(host.camera().x, -16);
    });

    test("maximized windows take the working area, and fullscreen goes back to it", async () => {
        const host = await load({
            monitors: [{ name: "DP-1", workingArea: { x: 0, y: 30, w: 1280, h: 690 } }],
        });
        const [a, b] = openMany(host, 2);
        host.requestMode(b, "maximized");
        assert.equal(host.mode(b), "maximized");
        assert.deepEqual(host.screenRect(b), rect(0, 30, 1280, 690));

        host.requestMode(b, "fullscreen");
        assert.equal(host.mode(b), "fullscreen");
        assert.deepEqual(host.screenRect(b), rect(0, 0, 1280, 720));
        host.requestMode(b, "maximized", { fullscreen: false });
        assert.equal(host.mode(b), "maximized");
        assert.deepEqual(host.screenRect(b), rect(0, 30, 1280, 690));
        host.requestMode(b, "normal", { maximized: false });
        assert.equal(host.mode(b), "normal");
        assert.deepEqual(host.screenRect(b), rect(648, 46, 616, 658));
        assert.deepEqual(host.screenRect(a), rect(16, 46, 616, 658));
    });

    test("changing the width leaves maximized", async () => {
        const host = await load();
        const a = host.open();
        host.requestMode(a, "maximized");
        host.action("set-column-width", px(500));
        assert.equal(host.mode(a), "normal");
        assert.deepEqual(host.rect(a), rect(0, 16, 500, 688));
    });

    test("windows can open fullscreen", async () => {
        const host = await load();
        host.open();
        const id = host.open({ mode: "fullscreen" });
        assert.deepEqual(host.screenRect(id), rect(0, 0, 1280, 720));
        assert.equal(host.focused(), id);
    });
});

describe("dragging windows", () => {
    test("dropping between columns makes a new column", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-first");
        // Near the gap between a and b.
        const pointer = { x: 620, y: 300 };
        assert.deepEqual(hintAt(host, c, pointer), rect(474, 16, 300, 688));
        drop(host, c, pointer);
        assert.deepEqual(columnsOf(host), [[a], [c], [b]]);
        assert.deepEqual(host.rect(c), rect(632, 16, 616, 688));
        assert.equal(host.focused(), c);
    });

    test("dropping past the last column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        const info = startDrag(host, b);
        dragOver(host, info, { x: 620, y: 300 }, undefined, true);
        assert.deepEqual(hintOf(host).rect, rect(632, 16, 300, 688));
        // Before the first column; the hint stays half on screen.
        dragOver(host, info, { x: -100, y: 300 });
        assert.deepEqual(hintOf(host).rect, rect(-166, 16, 300, 688));
        host.send({ type: "dragleave", workspace: host.activeWorkspace().id });
        drop(host, b, { x: 620, y: 300 });
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
    });

    test("dropping between tiles puts the window in the column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        const pointer = { x: 300, y: 700 };
        assert.deepEqual(hintAt(host, b, pointer), rect(0, 554, 616, 150));
        drop(host, b, pointer);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(b), rect(0, 368, 616, 336));
        assert.equal(host.focused(), b);
    });

    test("a dropped window keeps its column width", async () => {
        const host = await load();
        const a = host.open();
        const b = host.open(wide({ proportion: 1 / 3 }));
        host.drag(b, host.activeWorkspace().id, [{ x: -100, y: 300 }]);
        assert.deepEqual(host.rect(b), rect(0, 16, 405, 688));
        assert.deepEqual(host.rect(a), rect(421, 16, 616, 688));
    });

    test("dropping on an empty workspace", async () => {
        const host = await load();
        const a = host.open();
        // On screen where the new column will be.
        assert.deepEqual(hintAt(host, a, { x: 500, y: 300 }), rect(0, 16, 300, 688));
        assert.equal(host.camera().x, -16);
        drop(host, a, { x: 500, y: 300 });
        assert.deepEqual(host.screenRect(a), rect(16, 16, 616, 688));
    });

    test("the hint goes away when the pointer leaves, and follows its options", async () => {
        const host = await load();
        const [, b] = openMany(host, 2);
        const info = startDrag(host, b);
        dragOver(host, info, { x: 620, y: 300 }, undefined, true);
        assert.notEqual(hintOf(host), null);
        host.send({ type: "dragleave", workspace: host.activeWorkspace().id });
        assert.equal(hintOf(host), null);

        const plain = await load({ options: { "insert-hint": false } });
        const [, c] = openMany(plain, 2);
        dragOver(plain, startDrag(plain, c), { x: 620, y: 300 }, undefined, true);
        assert.equal(drawn(plain).length, 0);

        const colored = await load({ options: { "insert-hint-color": "#ff000080" } });
        const [, d] = openMany(colored, 2);
        dragOver(colored, startDrag(colored, d), { x: 620, y: 300 }, undefined, true);
        assert.equal(drawn(colored)[0].background, "#ff000080");

        // A gradient wins over the color.
        const gradient = { from: "#ff0000", to: "#0000ff", angle: 45, relativeTo: "view" };
        const painted = await load({
            options: { "insert-hint-color": "#ff000080", "insert-hint-gradient": gradient },
        });
        const [, e] = openMany(painted, 2);
        dragOver(painted, startDrag(painted, e), { x: 620, y: 300 }, undefined, true);
        assert.deepEqual(drawn(painted)[0].background, gradient);
    });
});

describe("interactive resize", () => {
    test("resizing the right edge changes the width", async () => {
        const host = await load();
        const a = host.open();
        assert.equal(host.resize(a, { right: true }, { x: 100, y: 0 }), true);
        assert.deepEqual(host.rect(a), rect(0, 16, 716, 688));
        assert.equal(host.camera().x, -16);
    });

    test("resizing the left edge keeps the right edge in place", async () => {
        const host = await load();
        const [, b] = openMany(host, 2);
        host.resize(b, { left: true }, { x: -100, y: 0 });
        assert.deepEqual(host.rect(b), rect(632, 16, 716, 688));
        assert.deepEqual(host.screenRect(b), rect(548, 16, 716, 688));
    });

    test("resizing the bottom edge changes the height", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.resize(a, { bottom: true }, { x: 0, y: 100 });
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 436));
        assert.deepEqual(host.rect(b), rect(0, 468, 616, 236));
        // The top edge of the top window doesn't resize.
        host.resize(a, { top: true }, { x: 0, y: 50 });
        assert.equal(host.rect(a).h, 436);
    });

    test("centered columns resize from both sides", async () => {
        const host = await load({ options: { "center-focused-column": "always" } });
        const a = host.open();
        host.resize(a, { right: true }, { x: 50, y: 0 });
        assert.deepEqual(host.screenRect(a), rect(282, 16, 716, 688));
    });

    test("fullscreen windows don't resize", async () => {
        const host = await load();
        const a = host.open();
        host.requestMode(a, "fullscreen");
        assert.equal(host.resize(a, { right: true }, { x: 100, y: 0 }), false);
    });
});

describe("swipes", () => {
    test("the view snaps to column edges and focuses what's in view", async () => {
        const host = await load();
        const [a, b] = openMany(host, 3);
        assert.equal(host.camera().x, 616);
        host.swipe(host.activeWorkspace().id, [-600]);
        assert.equal(host.camera().x, -16);
        assert.equal(host.focused(), b);

        host.action("focus-column-last");
        host.swipe(host.activeWorkspace().id, [-50], -2000);
        assert.equal(host.camera().x, -16);
        assert.equal(host.focused(), a);
    });

    test("with centered columns, the view snaps to the closest centered column", async () => {
        const host = await load({ options: { "center-focused-column": "always" } });
        const [, b] = openMany(host, 3);
        assert.equal(host.camera().x, 932);
        host.swipe(host.activeWorkspace().id, [-700]);
        assert.equal(host.focused(), b);
        assert.deepEqual(host.screenRect(b), rect(332, 16, 616, 688));
    });

    test("small swipes snap back", async () => {
        const host = await load();
        const [, , c] = openMany(host, 3);
        host.swipe(host.activeWorkspace().id, [-100]);
        assert.equal(host.camera().x, 616);
        assert.equal(host.focused(), c);
    });
});

describe("reloading the engine", () => {
    test("reloading gives back the same layout", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.action("set-window-height", px(200));
        const c = host.open();
        host.action("switch-preset-column-width");
        const d = host.open();
        const e = host.open();
        host.action("consume-or-expel-window-left");
        host.action("toggle-column-tabbed-display");
        const before = { format: host.format(), camera: host.camera(), focused: host.focused() };

        host.reload();
        assert.deepEqual(
            { format: host.format(), camera: host.camera(), focused: host.focused() },
            before,
        );
        assert.deepEqual(host.errors, []);

        // State that isn't in the rects comes back too: the preset width index, and going back
        // to the previous column when closing a new window.
        host.action("focus-column-left");
        host.action("switch-preset-column-width");
        assert.equal(host.rect(c).w, 405);
        host.action("focus-column-right");
        const f = host.open();
        host.reload();
        host.close(f);
        assert.equal(host.focused(), e);
        assert.deepEqual(
            [a, b, d].map((id) => position(host, id)),
            [
                [1, 1],
                [1, 2],
                [3, 1],
            ],
        );
    });

    test("without saved state, windows are put back in columns from where they were", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        const c = host.open(wide({ proportion: 1 / 3 }));
        const d = host.open();
        host.action("focus-column-left");
        const before = host.format();

        // The windows come with where they were, and the engine puts stacked ones into columns
        // again.
        host.reload({ state: false });
        assert.deepEqual(host.errors, []);
        assert.equal(host.format(), before);
        assert.equal(host.focused(), c);
        assert.deepEqual(
            [a, b, c, d].map((id) => position(host, id)),
            [
                [1, 1],
                [1, 2],
                [2, 1],
                [3, 1],
            ],
        );
        // The width was recognized as a preset.
        host.action("switch-preset-column-width");
        assert.equal(host.rect(c).w, 616);
    });
});

describe("windows that change", () => {
    test("a window with a max height leaves space below it", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.setLimits(a, { maxSize: { w: 0, h: 200 } });
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 200));
        assert.deepEqual(host.rect(b), rect(0, 232, 616, 336));
    });

    test("a wider min width moves the next columns, and the view follows the focus", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.setLimits(a, { minSize: { w: 700, h: 0 } });
        assert.deepEqual(host.rect(a), rect(0, 16, 700, 688));
        assert.deepEqual(host.rect(b), rect(716, 16, 616, 688));
        assert.deepEqual(host.screenRect(b), rect(648, 16, 616, 688));
    });

    test("a window that doesn't take its size is laid out with the size it took", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        const win = host.windows.get(a);
        win.stubborn = { w: 600, h: 680 };
        host.flush();
        assert.deepEqual(host.rect(a), rect(0, 16, 600, 680));
        assert.deepEqual(host.rect(b), rect(616, 16, 616, 688));
        // Asking for another size asks again.
        delete win.stubborn;
        host.actionOn(a, "set-window-width", px(500));
        assert.deepEqual(host.rect(a), rect(0, 16, 500, 688));
    });

    test("the output size changes column sizes", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.resizeMonitor("DP-1", { size: { w: 1920, h: 1080 } });
        assert.deepEqual(host.rect(a), rect(0, 16, 936, 1048));
        assert.deepEqual(host.rect(b), rect(952, 16, 936, 1048));
        // The focused column stays where it was on screen, since it still fits.
        assert.deepEqual(host.screenRect(b), rect(648, 16, 936, 1048));
    });

    test("config changes apply right away", async () => {
        const host = await load();
        const a = host.open();
        host.config = { ...host.config, gaps: 8 };
        host.setOptions({});
        assert.deepEqual(host.rect(a), rect(0, 8, 628, 704));
        // It's still in view, so the view doesn't move.
        assert.deepEqual(host.screenRect(a), rect(16, 8, 628, 704));
        host.action("center-column");
        host.action("focus-column-first");
        assert.deepEqual(host.screenRect(a), rect(326, 8, 628, 704));
    });
});

describe("queries", () => {
    test("new window sizes", async () => {
        const host = await load();
        assert.deepEqual(host.newWindowSize(), { w: 616, h: 688 });
        assert.deepEqual(
            host.newWindowSize(undefined, { rules: { "default-column-width": null } }),
            { w: 0, h: 688 },
        );
        assert.deepEqual(
            host.newWindowSize(undefined, {
                rules: { "default-window-height": { proportion: 0.5 } },
            }),
            { w: 616, h: 336 },
        );
        assert.deepEqual(
            host.newWindowSize(undefined, {
                rules: { "default-column-width": { fixed: 500 } },
                borders: { w: 8, h: 8 },
            }),
            { w: 508, h: 688 },
        );
        // Fixed-size windows get their size.
        assert.deepEqual(
            host.newWindowSize(undefined, {
                minSize: { w: 300, h: 200 },
                maxSize: { w: 300, h: 200 },
            }),
            { w: 300, h: 200 },
        );
    });

    test("a workspace's size comes from its own monitor", async () => {
        const host = await load({
            monitors: [{ name: "DP-1" }, { name: "DP-2", size: { w: 1920, h: 1080 } }],
        });
        assert.deepEqual(host.newWindowSize(host.workspace(0, "DP-2").id), { w: 936, h: 1048 });
        assert.deepEqual(host.newWindowSize(host.workspace(0, "DP-1").id), { w: 616, h: 688 });
    });

    test("the engine exposes its columns", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        const c = host.open();
        const [first, second] = host.exposedState().workspaces;
        assert.equal(first.id, host.workspace(0).id);
        assert.equal(first.active, 1);
        assert.deepEqual(first.columns, [
            {
                windows: [a, b],
                active: b,
                width: { proportion: 0.5 },
                fullWidth: false,
                display: "normal",
                mode: "normal",
            },
            {
                windows: [c],
                active: c,
                width: { proportion: 0.5 },
                fullWidth: false,
                display: "normal",
                mode: "normal",
            },
        ]);
        assert.deepEqual(second.columns, []);
        assert.deepEqual(columnsOf(host), [[a, b], [c]]);

        // Those watching hear about changes, and only about changes.
        const changes = host.stateChanges;
        host.action("focus-column-right");
        assert.equal(host.stateChanges, changes);
        host.action("focus-column-left");
        assert.ok(host.stateChanges > changes);
    });

    test("unknown actions do nothing", async () => {
        const host = await load();
        const a = host.open();
        host.action("not-an-action");
        host.action("set-column-width", "bogus");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
    });
});

describe("window rules", () => {
    test("open-maximized opens a column as wide as the view", async () => {
        const host = await load();
        const a = host.open({ rules: { "open-maximized": true } });
        assert.deepEqual(host.rect(a), rect(0, 16, 1248, 688));
    });

    test("default-column-display makes a new column tabbed", async () => {
        const host = await load();
        host.open({ rules: { "default-column-display": "tabbed" } });
        host.open();
        host.action("consume-or-expel-window-left");
        assert.deepEqual(columnsOf(host)[0].length, 2);
        assert.equal(host.exposedState().workspaces[0].columns[0].display, "tabbed");
    });

    test("a rule that leaves the width empty gets the option's width for a window without a size", async () => {
        const host = await load();
        const a = host.open({ rules: { "default-column-width": null } });
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
    });
});

describe("options", () => {
    test("default-column-width and the presets are the engine's own", async () => {
        const host = await load({
            options: {
                "default-column-width": "800",
                "preset-column-widths": ["25%", "75%"],
            },
        });
        const a = host.open();
        assert.equal(host.rect(a).w, 800);
        host.action("switch-preset-column-width");
        assert.equal(host.rect(a).w, 932);
        host.action("switch-preset-column-width");
        assert.equal(host.rect(a).w, 300);
    });

    test("changing the presets resets the columns' preset index", async () => {
        const host = await load();
        const a = host.open();
        host.setOptions({ "preset-column-widths": ["40%", "60%"] });
        host.action("switch-preset-column-width");
        // 616 px isn't one of the new presets: the next larger one is 60%.
        assert.equal(host.rect(a).w, 742);
    });
});

describe("workspaces and monitors", () => {
    test("each workspace has columns of its own", async () => {
        const host = await load();
        const [w0, w1] = [host.workspace(0).id, host.workspace(1).id];
        const a = host.open();
        host.showWorkspace(w1);
        const b = host.open();
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.deepEqual(columnsOf(host, w0), [[a]]);
        assert.deepEqual(columnsOf(host, w1), [[b]]);
        assert.deepEqual(host.camera(w1), { x: -16, y: 0, zoom: 1 });
    });

    test("move-column-to-workspace-down takes the column and focus along", async () => {
        const host = await load();
        const [w0, w1] = [host.workspace(0).id, host.workspace(1).id];
        const [a, b] = openMany(host, 2);
        host.action("move-column-to-workspace-down");
        assert.deepEqual(columnsOf(host, w0), [[a]]);
        assert.deepEqual(columnsOf(host, w1), [[b]]);
        assert.equal(host.windows.get(b).workspace, w1);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.equal(host.activeWorkspace().id, w1);
        assert.equal(host.focused(), b);
        assert.equal(host.workspace(0).focused, a);

        host.action("move-column-to-workspace-up");
        assert.deepEqual(columnsOf(host, w0), [[a], [b]]);
        assert.deepEqual(columnsOf(host, w1), []);
        assert.equal(host.activeWorkspace().id, w0);
        assert.equal(host.focused(), b);
    });

    test("move-column-to-workspace moves a stacked column whole, and focus can stay", async () => {
        const host = await load();
        const [w0, w1] = [host.workspace(0).id, host.workspace(1).id];
        const c = host.open();
        const [a, b] = openStacked(host);
        host.action("move-column-to-workspace", w1, false);
        assert.deepEqual(columnsOf(host, w0), [[c]]);
        assert.deepEqual(columnsOf(host, w1), [[a, b]]);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(b), rect(0, 368, 616, 336));
        // Focus stays on this workspace, on the window that's left.
        assert.equal(host.activeWorkspace().id, w0);
        assert.equal(host.focused(), c);
    });

    test("moving a column to the workspace it's on does nothing", async () => {
        const host = await load();
        const [a] = openMany(host, 2);
        host.action("move-column-to-workspace", host.workspace(0).id);
        assert.deepEqual(columnsOf(host)[0], [a]);
    });

    test("focus-window-or-workspace-down goes to the workspace below", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.action("focus-window-or-workspace-down");
        assert.equal(host.activeWorkspace().id, host.workspace(1).id);
        host.action("focus-window-or-workspace-up");
        assert.equal(host.activeWorkspace().id, host.workspace(0).id);
        assert.equal(host.focused(), b);
        host.action("focus-window-or-workspace-up");
        assert.equal(host.focused(), a);
        host.action("focus-window-or-workspace-up");
        assert.equal(host.activeWorkspace().id, host.workspace(0).id);
    });

    test("move-window-down-or-to-workspace-down leaves the column from its bottom", async () => {
        const host = await load();
        const [w0, w1] = [host.workspace(0).id, host.workspace(1).id];
        const [a, b] = openStacked(host);
        host.action("move-window-up-or-to-workspace-up");
        assert.deepEqual(columnsOf(host, w0), [[b, a]]);
        host.action("move-window-down-or-to-workspace-down");
        assert.deepEqual(columnsOf(host, w0), [[a, b]]);
        host.action("move-window-down-or-to-workspace-down");
        assert.deepEqual(columnsOf(host, w0), [[a]]);
        assert.deepEqual(columnsOf(host, w1), [[b]]);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.equal(host.activeWorkspace().id, w1);
        assert.equal(host.focused(), b);
    });

    test("a window that bento moved gets a column of its own", async () => {
        const host = await load();
        const [w0, w1] = [host.workspace(0).id, host.workspace(1).id];
        const [a, b] = openMany(host, 2);
        host.move(b, w1);
        assert.deepEqual(columnsOf(host, w0), [[a]]);
        assert.deepEqual(columnsOf(host, w1), [[b]]);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.equal(host.focused(), a);
    });

    test("focus crosses to the monitor beside when a strip's edge is reached", async () => {
        const host = await load({ monitors: [{ name: "DP-1" }, { name: "DP-2" }] });
        const a = host.open();
        const b = host.open({ workspace: host.activeWorkspace("DP-2").id });
        host.action("focus-column-or-monitor-right");
        assert.equal(host.focusedMonitor, "DP-2");
        assert.equal(host.focused(), b);
        host.action("focus-column-or-monitor-right");
        assert.equal(host.focusedMonitor, "DP-2");
        host.action("focus-column-or-monitor-left");
        assert.equal(host.focusedMonitor, "DP-1");
        assert.equal(host.focused(), a);

        // Inside a strip, the same actions move between columns.
        const c = host.open();
        host.action("focus-column-or-monitor-left");
        assert.equal(host.focusedMonitor, "DP-1");
        assert.equal(host.focused(), a);
        host.action("focus-window-or-monitor-up");
        assert.equal(host.focusedMonitor, "DP-1");
        assert.deepEqual(columnsOf(host), [[a], [c]]);
    });

    test("workspaces follow the options of their monitor and their own name", async () => {
        const host = await load({
            monitors: [{ name: "DP-1" }, { name: "DP-2" }],
            outputOptions: { "DP-2": { "default-column-width": "25%", "tab-indicator-width": 8 } },
        });
        const [dp1, dp2] = [host.workspace(0, "DP-1"), host.workspace(0, "DP-2")];
        const a = host.open();
        const b = host.open({ workspace: dp2.id });
        assert.equal(host.rect(a).w, 616);
        assert.equal(host.rect(b).w, 300);

        // The indicator is drawn with the options of the workspace it's on.
        host.actionOn(a, "toggle-column-tabbed-display");
        host.actionOn(b, "toggle-column-tabbed-display");
        assert.equal(tabsOf(host, dp1.id)[0].rect.w, 4);
        assert.equal(tabsOf(host, dp2.id)[0].rect.w, 8);

        // A workspace's own options come after its monitor's, by name.
        const web = host.workspace(1, "DP-2");
        web.name = "web";
        host.setWorkspaceOptions("web", { "default-column-width": "100%" });
        const c = host.open({ workspace: web.id });
        assert.equal(host.rect(c).w, 1248);
        // Changing them reaches the strips that already exist.
        host.setOutputOptions("DP-2", { "tab-indicator-width": 6 });
        assert.equal(tabsOf(host, dp2.id)[0].rect.w, 6);
        assert.equal(tabsOf(host, dp1.id)[0].rect.w, 4);
    });

    test("columns move to the monitor beside, laid out for its size", async () => {
        const host = await load({
            monitors: [{ name: "DP-1" }, { name: "DP-2", size: { w: 1920, h: 1080 } }],
        });
        const [dp1, dp2] = [host.workspace(0, "DP-1").id, host.workspace(0, "DP-2").id];
        const [a, b] = openMany(host, 2);
        // The last column goes on to the monitor beside.
        host.action("move-column-right-or-to-monitor-right");
        assert.deepEqual(columnsOf(host, dp1), [[a]]);
        assert.deepEqual(columnsOf(host, dp2), [[b]]);
        assert.deepEqual(host.rect(b), rect(0, 16, 936, 1048));
        assert.equal(host.focusedMonitor, "DP-2");
        assert.equal(host.focused(), b);

        // The first one comes back, right of the focused column.
        host.action("move-column-left-or-to-monitor-left");
        assert.deepEqual(columnsOf(host, dp1), [[a], [b]]);
        assert.deepEqual(columnsOf(host, dp2), []);
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.equal(host.focusedMonitor, "DP-1");

        // In the middle of a strip they move within it.
        host.action("move-column-left-or-to-monitor-left");
        assert.deepEqual(columnsOf(host, dp1), [[b], [a]]);
    });

    test("move-column-to-monitor names a monitor, and focus can stay", async () => {
        const host = await load({ monitors: [{ name: "DP-1" }, { name: "DP-2" }] });
        const [a, b] = openMany(host, 2);
        host.action("move-column-to-monitor", "DP-2", false);
        assert.deepEqual(columnsOf(host, host.workspace(0, "DP-1").id), [[a]]);
        assert.deepEqual(columnsOf(host, host.workspace(0, "DP-2").id), [[b]]);
        assert.equal(host.focusedMonitor, "DP-1");
        assert.equal(host.focused(), a);

        host.action("move-column-to-monitor-previous");
        assert.deepEqual(columnsOf(host, host.workspace(0, "DP-1").id), []);
        assert.deepEqual(columnsOf(host, host.workspace(0, "DP-2").id), [[b], [a]]);
        assert.equal(host.focusedMonitor, "DP-2");
        host.action("move-column-to-monitor-next");
        assert.deepEqual(columnsOf(host, host.workspace(0, "DP-1").id), [[a]]);
    });

    test("a monitor's size change only lays out its own workspaces", async () => {
        const host = await load({ monitors: [{ name: "DP-1" }, { name: "DP-2" }] });
        const a = host.open();
        const b = host.open({ workspace: host.activeWorkspace("DP-2").id });
        host.resizeMonitor("DP-2", { size: { w: 1920, h: 1080 } });
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(0, 16, 936, 1048));
    });
});

describe("edge double clicks", () => {
    test("left and right edges toggle the column's full width", async () => {
        const host = await load();
        const [, b] = openMany(host, 2);
        host.doubleClickEdge(b, { right: true });
        assert.equal(host.rect(b).w, 1248);
        host.doubleClickEdge(b, { left: true });
        assert.equal(host.rect(b).w, 616);
    });

    test("top and bottom edges toggle the window's full height", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.doubleClickEdge(a, { bottom: true });
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 671));
        assert.equal(host.rect(b).h, 1);
        host.doubleClickEdge(a, { top: true });
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(b), rect(0, 368, 616, 336));
    });
});

// Groups are bento's, and the host keeps them like bento does: `groupWindows` is bento's own
// action that groups windows, and `groupOf` the group a window is in.
describe("groups", () => {
    test("a tabbed column is a group, and stacking it dissolves the group", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        assert.equal(host.groupOf(a), null);
        host.action("toggle-column-tabbed-display");
        assert.deepEqual(host.groupOf(a).windows, [a, b]);
        assert.equal(host.groupOf(a).active, b);
        host.action("focus-window-up");
        assert.equal(host.groupOf(a).active, a);
        host.action("set-column-display", "normal");
        assert.equal(host.groupOf(a), null);
        assert.equal(host.groupOf(b), null);
    });

    test("a group made elsewhere shows as a tabbed column where its active window was", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.groupWindows([a, c], c);
        assert.deepEqual(columnsOf(host), [[b], [c, a]]);
        assert.equal(host.exposedState().workspaces[0].columns[1].display, "tabbed");
        assert.deepEqual(host.rect(a), host.rect(c));
        assert.equal(host.isVisible(c), true);
        assert.equal(host.isVisible(a), false);
        assert.equal(tabsOf(host).length, 2);
    });

    test("a window that leaves a tabbed column leaves its group", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        const c = host.open();
        host.action("consume-or-expel-window-left");
        host.action("toggle-column-tabbed-display");
        assert.deepEqual(host.groupOf(a).windows, [a, b, c]);
        host.action("expel-window-from-column");
        assert.deepEqual(columnsOf(host), [[a, b], [c]]);
        assert.deepEqual(host.groupOf(a).windows, [a, b]);
        assert.equal(host.groupOf(c), null);
    });
});

describe("random operations", () => {
    const actions = [
        ["focus-column-left"],
        ["focus-column-right"],
        ["focus-column-first"],
        ["focus-column-last"],
        ["focus-column-right-or-first"],
        ["focus-column-left-or-last"],
        ["focus-column", 2],
        ["focus-window-in-column", 2],
        ["focus-window-down"],
        ["focus-window-up"],
        ["focus-window-down-or-column-left"],
        ["focus-window-down-or-column-right"],
        ["focus-window-up-or-column-left"],
        ["focus-window-up-or-column-right"],
        ["focus-window-top"],
        ["focus-window-bottom"],
        ["focus-window-down-or-top"],
        ["focus-window-up-or-bottom"],
        ["move-column-left"],
        ["move-column-right"],
        ["move-column-to-first"],
        ["move-column-to-last"],
        ["move-column-to-index", 2],
        ["move-window-down"],
        ["move-window-up"],
        ["consume-or-expel-window-left"],
        ["consume-or-expel-window-right"],
        ["consume-window-into-column"],
        ["expel-window-from-column"],
        ["swap-window-left"],
        ["swap-window-right"],
        ["toggle-column-tabbed-display"],
        ["set-column-display", "tabbed"],
        ["center-column"],
        ["center-window"],
        ["center-visible-columns"],
        ["switch-preset-column-width"],
        ["switch-preset-column-width-back"],
        ["switch-preset-window-width"],
        ["switch-preset-window-height"],
        ["switch-preset-window-height-back"],
        ["maximize-column"],
        ["set-column-width", adjust(-10, "%")],
        ["set-column-width", px(300)],
        ["set-window-width", adjust(50, "px")],
        ["set-window-height", adjust(-100, "px")],
        ["set-window-height", percent(40)],
        ["reset-window-height"],
        ["expand-column-to-available-width"],
    ];

    /** Actions that cross to other workspaces and monitors. */
    const crossing = [
        ["focus-column-or-monitor-left"],
        ["focus-column-or-monitor-right"],
        ["focus-window-or-monitor-up"],
        ["focus-window-or-monitor-down"],
        ["focus-window-or-workspace-up"],
        ["focus-window-or-workspace-down"],
        ["move-column-left-or-to-monitor-left"],
        ["move-column-right-or-to-monitor-right"],
        ["move-column-to-workspace-up"],
        ["move-column-to-workspace-down", false],
        ["move-column-to-monitor-left"],
        ["move-column-to-monitor-right", false],
        ["move-column-to-monitor-previous"],
        ["move-column-to-monitor-next"],
        ["move-window-down-or-to-workspace-down"],
        ["move-window-up-or-to-workspace-up"],
    ];

    /** A small deterministic random number generator, so failures can be replayed. */
    function random(seed) {
        let state = seed;
        return (n) => {
            state = (state * 1103515245 + 12345) % 2147483648;
            return state % n;
        };
    }

    for (const [name, opts] of [
        ["default config", {}],
        [
            "centered, struts, fractional scale",
            {
                monitors: [
                    {
                        name: "DP-1",
                        scale: 1.5,
                        workingArea: { x: 0, y: 25, w: 1280, h: 695 },
                    },
                ],
                config: { struts: { left: 20.5, right: 10, top: 5, bottom: 0 } },
                options: {
                    "center-focused-column": "always",
                    "tab-indicator-position": "top",
                    "tab-indicator-width": 4,
                    "tab-indicator-gap": 5,
                    "tab-indicator-place-within-column": true,
                    "tab-indicator-hide-when-single-tab": true,
                },
            },
        ],
        [
            "on-overflow, single column centered",
            {
                config: { gaps: 7 },
                options: {
                    "center-focused-column": "on-overflow",
                    "always-center-single-column": true,
                },
            },
        ],
    ]) {
        test(`layouts stay consistent: ${name}`, async () => {
            const host = await load(opts);
            const monitor = host.monitors.get("DP-1");
            for (let seed = 1; seed <= 20; seed++) {
                const next = random(seed);
                const ids = () => [...host.windows.keys()];
                const pick = () => ids()[next(ids().length)];
                for (let step = 0; step < 60; step++) {
                    const op = next(14);
                    const any = ids().length > 0;
                    if (op <= 2 || !any) {
                        const limits = next(4) === 0 ? { minSize: { w: 300, h: 150 } } : {};
                        host.open({
                            borders: next(2) ? { w: 4, h: 4 } : { w: 0, h: 0 },
                            ...limits,
                        });
                    } else if (op === 3) {
                        host.close(pick());
                    } else if (op <= 7) {
                        const [action, ...args] = actions[next(actions.length)];
                        host.action(action, ...args);
                    } else if (op === 8) {
                        const [action, ...args] = actions[next(actions.length)];
                        host.actionOn(pick(), action, ...args);
                    } else if (op === 9) {
                        host.requestMode(pick(), ["normal", "fullscreen", "maximized"][next(3)]);
                    } else if (op === 10) {
                        host.resize(
                            pick(),
                            { left: !!next(2), bottom: !!next(2) },
                            { x: next(400) - 200, y: next(300) - 150 },
                        );
                    } else if (op === 11) {
                        host.swipe(host.activeWorkspace().id, [next(1600) - 800]);
                    } else if (op === 12) {
                        host.drag(pick(), host.activeWorkspace().id, [
                            { x: host.camera().x + next(1280), y: next(720) },
                        ]);
                    } else {
                        host.reload();
                    }
                    assert.deepEqual(host.errors, []);
                    assert.deepEqual(host.logs, [], `seed ${seed} step ${step}`);
                    for (const id of ids()) assert.ok(host.rect(id), `window ${id} has a rect`);
                    if (host.focused() !== null) {
                        assert.equal(host.isVisible(host.focused()), true);
                        // The view always scrolls to show the focused window, or its left
                        // edge if it's wider than the working area.
                        const screen = host.screenRect(host.focused());
                        const { left, right } = host.config.struts;
                        const fits = screen.w <= monitor.workingArea.w - left - right;
                        assert.ok(
                            screen.x >= -1 && (!fits || screen.x + screen.w <= monitor.size.w + 1),
                            `seed ${seed} step ${step}: focused window at ${JSON.stringify(screen)}`,
                        );
                    }
                }
                for (const id of ids()) host.close(id);
            }
        });
    }

    test("windows stay in one strip across workspaces and monitors", async () => {
        const host = await load({
            monitors: [
                { name: "DP-1", workspaces: 3 },
                { name: "DP-2", size: { w: 1920, h: 1080 }, workspaces: 2 },
            ],
        });
        const workspaces = () => [...host.workspaces.keys()];
        for (let seed = 1; seed <= 20; seed++) {
            const next = random(seed);
            const ids = () => [...host.windows.keys()];
            const pick = () => ids()[next(ids().length)];
            for (let step = 0; step < 60; step++) {
                const op = next(12);
                const any = ids().length > 0;
                if (op <= 2 || !any) {
                    host.open({ workspace: workspaces()[next(workspaces().length)] });
                } else if (op === 3) {
                    host.close(pick());
                } else if (op <= 6) {
                    const [action, ...args] = crossing[next(crossing.length)];
                    host.action(action, ...args);
                } else if (op === 7) {
                    const [action, ...args] = actions[next(actions.length)];
                    host.action(action, ...args);
                } else if (op === 8) {
                    host.move(pick(), workspaces()[next(workspaces().length)]);
                } else if (op === 9) {
                    host.showWorkspace(workspaces()[next(workspaces().length)]);
                } else if (op === 10) {
                    host.action("move-column-to-workspace", workspaces()[next(workspaces().length)]);
                } else {
                    host.reload();
                }
                assert.deepEqual(host.errors, []);
                assert.deepEqual(host.logs, [], `seed ${seed} step ${step}`);
                const seen = [];
                for (const ws of host.exposedState().workspaces) {
                    for (const id of ws.columns.flatMap((col) => col.windows)) {
                        assert.equal(host.windows.get(id).workspace, ws.id, `seed ${seed} step ${step}`);
                        seen.push(id);
                    }
                }
                assert.deepEqual(seen.sort(), ids().sort(), `seed ${seed} step ${step}`);
                for (const id of ids()) assert.ok(host.rect(id), `window ${id} has a rect`);
            }
            for (const id of ids()) host.close(id);
        }
    });
});
