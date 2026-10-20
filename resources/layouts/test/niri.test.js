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

/** Starts dragging a window: it leaves the layout. */
function startDrag(host, id) {
    host.send({ type: "remove", id, reason: "drag" });
    host.order = host.order.filter((other) => other !== id);
}

/** Drops a dragged window with the pointer at `pointer`. */
function drop(host, id, pointer) {
    const win = host.windows.get(id);
    host.send({
        type: "add",
        window: host.info(win),
        how: { reason: "drop", activate: true, rect: win.rect, pointer, nextTo: null },
    });
}

/** Creates the layout again from where the windows are, without saved state. */
function adoptFresh(host) {
    host.send({ type: "destroy" });
    host.instance = TestHost.nextInstance++;
    host.send({
        type: "create",
        view: host.view,
        camera: host.camera,
        config: host.config,
        options: host.options,
        windows: host.order.map((id) => host.info(host.windows.get(id))),
        focused: host.focused,
        state: null,
    });
}

describe("opening windows", () => {
    test("the first window fills the height with the default width", async () => {
        const host = await load();
        const a = host.open();
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.camera, { x: -16, y: 0 });
        assert.deepEqual(host.screenRect(a), rect(16, 16, 616, 688));
        assert.equal(host.focused, a);
        assert.deepEqual(host.errors, []);
    });

    test("new windows open right of the focused column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        // Both fit, so the view doesn't move.
        assert.equal(host.camera.x, -16);

        // The third doesn't fit: the view scrolls the least it can to show it.
        const c = host.open();
        assert.deepEqual(host.rect(c), rect(1264, 16, 616, 688));
        assert.equal(host.camera.x, 616);
        assert.deepEqual(host.screenRect(b), rect(16, 16, 616, 688));
        assert.deepEqual(host.screenRect(c), rect(648, 16, 616, 688));
        assert.equal(host.focused, c);

        // Opening from the first column puts the window right after it.
        host.action("focus-column-first");
        const d = host.open();
        assert.deepEqual(host.ipc.get(d), [2, 1]);
        assert.deepEqual(host.ipc.get(b), [3, 1]);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(d), rect(632, 16, 616, 688));
    });

    test("windows that don't activate don't take focus or move the view", async () => {
        const host = await load();
        const a = host.open();
        const b = host.open({ activate: false });
        assert.equal(host.focused, a);
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.equal(host.camera.x, -16);
    });

    test("windows open next to the window they belong to", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        const d = host.open({ nextTo: a });
        assert.deepEqual(
            [a, d, b, c].map((id) => host.ipc.get(id)[0]),
            [1, 2, 3, 4],
        );
        assert.equal(host.focused, d);
    });

    test("default width and height from the window rules", async () => {
        const host = await load();
        const a = host.open({ defaultWidth: { fixed: 500 }, defaultHeight: { proportion: 0.5 } });
        // 500 px is for the window; the tile has the borders too.
        const b = host.open({ defaultWidth: { fixed: 500 }, borders: { w: 8, h: 8 } });
        const c = host.open({ defaultWidth: { proportion: 1 / 3 } });
        assert.deepEqual(host.rect(a), rect(0, 16, 500, 336));
        assert.deepEqual(host.rect(b), rect(516, 16, 508, 688));
        assert.deepEqual(host.rect(c), rect(1040, 16, 405, 688));
    });

    test("struts and panels shrink the working area", async () => {
        const host = await load({
            workingArea: { x: 0, y: 30, w: 1280, h: 690 },
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
        assert.equal(host.focused, null);
        assert.deepEqual(host.screenRect(a), rect(16, 16, 616, 688));
    });

    test("windows that stop floating keep their width", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.send({ type: "remove", id: a, reason: "float" });
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        const win = host.windows.get(a);
        win.size = { w: 500, h: 400 };
        host.send({
            type: "add",
            window: host.info(win),
            how: {
                reason: "tile",
                activate: true,
                rect: rect(100, 100, 500, 400),
                pointer: null,
                nextTo: null,
            },
        });
        assert.deepEqual(host.rect(a), rect(632, 16, 500, 688));
        assert.equal(host.focused, a);
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
        assert.equal(host.camera.x, 616);
        host.close(c);
        assert.equal(host.focused, b);
        // The view goes back to where it was before the window opened.
        assert.equal(host.camera.x, -16);
    });

    test("after focus moves, closing the last column keeps the view", async () => {
        const host = await load();
        const [, b, c] = openMany(host, 3);
        host.action("focus-column-left");
        host.action("focus-column-right");
        host.close(c);
        assert.equal(host.focused, b);
        assert.equal(host.camera.x, 616);
        assert.deepEqual(host.screenRect(b), rect(16, 16, 616, 688));
    });

    test("the next column takes the place of a closed one", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-left");
        host.close(b);
        assert.equal(host.focused, c);
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
        assert.equal(host.focused, a);
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
        assert.equal(host.focused, b);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(c), rect(0, 368, 616, 336));
    });

    test("the first window on an emptied workspace resets the view", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        for (const id of [a, b, c]) host.close(id);
        assert.equal(host.focused, null);
        const d = host.open();
        assert.deepEqual(host.rect(d), rect(0, 16, 616, 688));
        assert.equal(host.camera.x, -16);
    });
});

describe("focus actions", () => {
    test("focusing columns", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        assert.equal(host.action("focus-column-right"), false);
        assert.equal(host.action("focus-column-left"), true);
        assert.equal(host.focused, b);
        // b is fully visible, so the view stays.
        assert.equal(host.camera.x, 616);
        host.action("focus-column-left");
        assert.equal(host.focused, a);
        assert.equal(host.camera.x, -16);
        assert.equal(host.action("focus-column-left"), false);
        host.action("focus-column-last");
        assert.equal(host.focused, c);
        host.action("focus-column-first");
        assert.equal(host.focused, a);
        host.action("focus-column-left-or-last");
        assert.equal(host.focused, c);
        host.action("focus-column-right-or-first");
        assert.equal(host.focused, a);
        host.action("focus-column", 2);
        assert.equal(host.focused, b);
        host.action("focus-column", 10);
        assert.equal(host.focused, c);
    });

    test("focusing windows in a column", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-first");
        host.action("consume-window-into-column");
        host.action("consume-window-into-column");
        assert.equal(host.focused, a);
        assert.equal(host.action("focus-window-up"), false);
        assert.equal(host.action("focus-window-down"), true);
        assert.equal(host.focused, b);
        host.action("focus-window-bottom");
        assert.equal(host.focused, c);
        assert.equal(host.action("focus-window-down"), false);
        host.action("focus-window-down-or-top");
        assert.equal(host.focused, a);
        host.action("focus-window-up-or-bottom");
        assert.equal(host.focused, c);
        host.action("focus-window-top");
        assert.equal(host.focused, a);
        host.action("focus-window-in-column", 2);
        assert.equal(host.focused, b);
        host.action("focus-window-in-column", 9);
        assert.equal(host.focused, c);
    });

    test("focusing windows or columns", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        const c = host.open();
        host.action("focus-column-left");
        assert.equal(host.focused, b);
        host.action("focus-window-down-or-column-right");
        assert.equal(host.focused, c);
        host.action("focus-window-up-or-column-left");
        assert.equal(host.focused, b);
        host.action("focus-window-up-or-column-right");
        assert.equal(host.focused, a);
        host.action("focus-window-down-or-column-left");
        assert.equal(host.focused, b);
        host.action("focus-window-down-or-column-left");
        assert.equal(host.focused, b);
    });

    test("activating a window focuses its column and tile", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        const c = host.open();
        host.activate(a);
        assert.equal(host.focused, a);
        assert.deepEqual(host.ipc.get(c), [2, 1]);
        host.activate(c);
        assert.equal(host.focused, c);
        host.activate(b);
        assert.equal(host.focused, b);
    });
});

describe("move actions", () => {
    test("moving columns", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        const order = () => [a, b, c].sort((x, y) => host.rect(x).x - host.rect(y).x);
        assert.equal(host.action("move-column-right"), false);
        assert.equal(host.action("move-column-left"), true);
        assert.deepEqual(order(), [a, c, b]);
        assert.equal(host.focused, c);
        host.action("move-column-to-first");
        assert.deepEqual(order(), [c, a, b]);
        assert.equal(host.camera.x, -16);
        assert.equal(host.action("move-column-left"), false);
        host.action("move-column-to-last");
        assert.deepEqual(order(), [a, b, c]);
        host.action("move-column-to-index", 2);
        assert.deepEqual(order(), [a, c, b]);
        assert.equal(host.focused, c);
        assert.deepEqual(host.ipc.get(c), [2, 1]);
    });

    test("moving windows in a column", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        assert.equal(host.action("move-window-down"), false);
        assert.equal(host.action("move-window-up"), true);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(a), rect(0, 368, 616, 336));
        assert.equal(host.focused, b);
        assert.equal(host.action("move-window-up"), false);
        assert.equal(host.action("move-window-down"), true);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 336));
    });
});

describe("consuming and expelling", () => {
    test("consume or expel left and right", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        assert.deepEqual(host.ipc.get(b), [1, 2]);
        assert.equal(host.focused, b);
        assert.equal(host.camera.x, -16);

        // Expelling right puts the window in a new column.
        host.action("consume-or-expel-window-right");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.equal(host.focused, b);

        // A single window can't go further right.
        assert.equal(host.action("consume-or-expel-window-right"), false);
        host.action("focus-column-left");
        host.action("consume-or-expel-window-right");
        assert.deepEqual(host.ipc.get(a), [1, 2]);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(a), rect(0, 368, 616, 336));
        assert.equal(host.focused, a);

        // Expelling left puts the window in a new column on the left.
        host.action("consume-or-expel-window-left");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.equal(host.focused, a);
        assert.equal(host.action("consume-or-expel-window-left"), false);
    });

    test("consume or expel a window by id", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.actionOn(a, "consume-or-expel-window-right");
        assert.deepEqual(host.ipc.get(a), [1, 2]);
        assert.deepEqual(host.ipc.get(c), [2, 1]);
        // The focused window stays focused.
        assert.equal(host.focused, c);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 336));
    });

    test("consume into and expel from the focused column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.action("focus-column-left");
        assert.equal(host.action("consume-window-into-column"), true);
        assert.equal(host.focused, a);
        assert.deepEqual(host.rect(b), rect(0, 368, 616, 336));
        assert.equal(host.action("consume-window-into-column"), false);
        assert.equal(host.action("expel-window-from-column"), true);
        assert.equal(host.focused, a);
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.equal(host.action("expel-window-from-column"), false);
    });

    test("swapping windows between columns", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-left");
        host.action("consume-window-into-column");
        // Columns: [a], [b, c] with b focused.
        host.action("swap-window-left");
        assert.equal(host.focused, b);
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(a), rect(632, 16, 616, 336));
        assert.deepEqual(host.rect(c), rect(632, 368, 616, 336));
        assert.equal(host.action("swap-window-left"), false);

        host.action("swap-window-right");
        assert.equal(host.focused, b);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 336));
    });

    test("swapping single windows moves the column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.action("swap-window-left");
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(a), rect(632, 16, 616, 688));
        assert.equal(host.focused, b);
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
        assert.equal(host.focused, b);
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
        assert.equal(host.camera.x, -16);
        host.action("maximize-column");
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
    });

    test("expand-column-to-available-width", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2, { defaultWidth: { proportion: 1 / 3 } });
        assert.deepEqual(host.rect(b), rect(421, 16, 405, 688));
        assert.equal(host.action("expand-column-to-available-width"), true);
        assert.deepEqual(host.rect(a), rect(0, 16, 405, 688));
        assert.deepEqual(host.rect(b), rect(421, 16, 827, 688));
        assert.equal(host.camera.x, -16);
        // Nothing left to expand into.
        assert.equal(host.action("expand-column-to-available-width"), false);

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
        assert.equal(host.actionOn(a, "center-window"), false);
        host.action("focus-column-left");
        assert.equal(host.actionOn(a, "center-window"), true);
        assert.deepEqual(host.screenRect(a), rect(332, 16, 616, 688));
    });

    test("center-visible-columns", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2, { defaultWidth: { proportion: 1 / 3 } });
        host.action("center-visible-columns");
        // 405 + 16 + 405 px of columns, centered in 1280 px.
        assert.deepEqual(host.screenRect(a), rect(227, 16, 405, 688));
        assert.deepEqual(host.screenRect(b), rect(648, 16, 405, 688));
    });

    test("center-focused-column always", async () => {
        const host = await load({ config: { centerFocusedColumn: "always" } });
        const a = host.open();
        assert.deepEqual(host.screenRect(a), rect(332, 16, 616, 688));
        const b = host.open();
        assert.deepEqual(host.screenRect(b), rect(332, 16, 616, 688));
        host.action("focus-column-left");
        assert.deepEqual(host.screenRect(a), rect(332, 16, 616, 688));
    });

    test("center-focused-column on-overflow", async () => {
        const host = await load({ config: { centerFocusedColumn: "on-overflow" } });
        const [, b] = openMany(host, 2);
        // Two half columns fit together.
        assert.deepEqual(host.screenRect(b), rect(648, 16, 616, 688));
        // A wide one doesn't fit with the previous one, so it's centered.
        const c = host.open({ defaultWidth: { proportion: 2 / 3 } });
        assert.deepEqual(host.screenRect(c), rect(227, 16, 826, 688));
    });

    test("always-center-single-column", async () => {
        const host = await load({ config: { alwaysCenterSingleColumn: true } });
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
        assert.equal(host.action("toggle-column-tabbed-display"), true);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.equal(host.isVisible(a), false);
        assert.equal(host.isVisible(b), true);

        host.action("focus-window-up");
        assert.equal(host.focused, a);
        assert.equal(host.isVisible(a), true);
        assert.equal(host.isVisible(b), false);

        // New tabs go in the background.
        const c = host.open();
        host.action("consume-or-expel-window-left");
        assert.equal(host.focused, c);
        assert.deepEqual(host.rect(c), rect(0, 16, 616, 688));
        assert.deepEqual(
            [a, b, c].map((id) => host.isVisible(id)),
            [false, false, true],
        );

        assert.equal(host.action("set-column-display", "tabbed"), false);
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

    test("tabbed columns show a tab bar", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        assert.equal(host.tabs.size, 0);
        host.action("toggle-column-tabbed-display");
        assert.equal(host.tabs.size, 1);
        const [bar] = host.tabs.values();
        assert.deepEqual(bar.windows, [a, b]);
        assert.equal(bar.active, b);
        assert.deepEqual(bar.area, rect(0, 16, 616, 688));

        // Clicking a tab switches to it.
        host.clickTab(bar.key, 0);
        assert.equal(host.focused, a);
        assert.equal(host.tabs.get(bar.key).active, a);
        assert.equal(host.isVisible(a), true);
        assert.equal(host.isVisible(b), false);

        // Fullscreen hides the bar.
        host.requestMode(a, "fullscreen");
        assert.equal(host.tabs.size, 0);
        host.requestMode(a, "normal");
        assert.equal(host.tabs.get(bar.key).active, a);

        host.action("set-column-display", "normal");
        assert.equal(host.tabs.size, 0);
    });

    test("a tab indicator within the column takes room from the tabs", async () => {
        const host = await load({
            config: {
                tabIndicator: {
                    off: false,
                    position: "left",
                    width: 4,
                    gap: 6,
                    placeWithinColumn: true,
                    hideWhenSingleTab: false,
                },
            },
        });
        const [a, b] = openStacked(host);
        host.action("toggle-column-tabbed-display");
        // 10 px for the indicator: the tabs are 10 px narrower and start 10 px to the right, and
        // the column keeps its width.
        assert.deepEqual(host.rect(b), rect(10, 16, 606, 688));
        const [bar] = host.tabs.values();
        assert.deepEqual(bar.area, rect(10, 16, 606, 688));
        const c = host.open();
        assert.deepEqual(host.rect(c), rect(632, 16, 616, 688));

        // Dropping into the column highlights the tabs, not the indicator.
        startDrag(host, c);
        assert.deepEqual(
            host.dropTarget(c, { x: 300, y: 700 }, host.rect(c)),
            rect(10, 554, 606, 150),
        );
        assert.deepEqual(host.rect(a), rect(10, 16, 606, 688));
    });

    test("new columns can be tabbed by default", async () => {
        const host = await load({ config: { defaultColumnDisplay: "tabbed" } });
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
        assert.equal(host.camera.x, -16);
    });

    test("fullscreen takes a window out of its column", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        host.requestMode(a, "fullscreen");
        assert.deepEqual(host.ipc.get(a), [2, 1]);
        assert.deepEqual(host.rect(a), rect(632, 0, 1280, 720));
        assert.deepEqual(host.rect(b), rect(0, 16, 616, 688));
        assert.equal(host.focused, b);
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
        assert.equal(host.camera.x, 0);
        // Leaving tabbed display leaves fullscreen.
        host.action("set-column-display", "normal");
        assert.deepEqual(
            [a, b].map((id) => host.mode(id)),
            ["normal", "normal"],
        );
        assert.equal(host.camera.x, -16);
    });

    test("maximized windows take the working area, and fullscreen goes back to it", async () => {
        const host = await load({ workingArea: { x: 0, y: 30, w: 1280, h: 690 } });
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
        const id = host.nextId;
        const win = host.makeWindow({});
        win.mode = "fullscreen";
        host.send({
            type: "add",
            window: host.info(win),
            how: { reason: "open", activate: true, rect: null, pointer: null, nextTo: null },
        });
        assert.deepEqual(host.screenRect(id), rect(0, 0, 1280, 720));
        assert.equal(host.focused, id);
    });
});

describe("dragging windows", () => {
    test("dropping between columns makes a new column", async () => {
        const host = await load();
        const [a, b, c] = openMany(host, 3);
        host.action("focus-column-first");
        startDrag(host, c);
        // Near the gap between a and b.
        const pointer = { x: 620, y: 300 };
        assert.deepEqual(host.dropTarget(c, pointer, host.rect(c)), rect(474, 16, 300, 688));
        drop(host, c, pointer);
        assert.deepEqual(
            [a, c, b].map((id) => host.ipc.get(id)),
            [
                [1, 1],
                [2, 1],
                [3, 1],
            ],
        );
        assert.deepEqual(host.rect(c), rect(632, 16, 616, 688));
        assert.equal(host.focused, c);
    });

    test("dropping past the last column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        startDrag(host, b);
        assert.deepEqual(
            host.dropTarget(b, { x: 620, y: 300 }, host.rect(b)),
            rect(632, 16, 300, 688),
        );
        // Before the first column; the hint stays half on screen.
        assert.deepEqual(
            host.dropTarget(b, { x: -100, y: 300 }, host.rect(b)),
            rect(-166, 16, 300, 688),
        );
        drop(host, b, { x: 620, y: 300 });
        assert.deepEqual(host.rect(b), rect(632, 16, 616, 688));
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
    });

    test("dropping between tiles puts the window in the column", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        startDrag(host, b);
        const pointer = { x: 300, y: 700 };
        assert.deepEqual(host.dropTarget(b, pointer, host.rect(b)), rect(0, 554, 616, 150));
        drop(host, b, pointer);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 336));
        assert.deepEqual(host.rect(b), rect(0, 368, 616, 336));
        assert.equal(host.focused, b);
    });

    test("a dropped window keeps its column width", async () => {
        const host = await load();
        const a = host.open();
        const b = host.open({ defaultWidth: { proportion: 1 / 3 } });
        host.drag(b, { x: -100, y: 300 }, host.rect(b));
        assert.deepEqual(host.rect(b), rect(0, 16, 405, 688));
        assert.deepEqual(host.rect(a), rect(421, 16, 616, 688));
    });

    test("dropping on an empty workspace", async () => {
        const host = await load();
        const a = host.open();
        startDrag(host, a);
        const hint = host.dropTarget(a, { x: 500, y: 300 }, host.rect(a));
        // On screen where the new column will be.
        assert.deepEqual(hint, rect(0, 16, 300, 688));
        assert.equal(host.camera.x, -16);
        drop(host, a, { x: 500, y: 300 });
        assert.deepEqual(host.screenRect(a), rect(16, 16, 616, 688));
    });
});

describe("interactive resize", () => {
    test("resizing the right edge changes the width", async () => {
        const host = await load();
        const a = host.open();
        assert.equal(host.resize(a, { right: true }, { x: 100, y: 0 }), true);
        assert.deepEqual(host.rect(a), rect(0, 16, 716, 688));
        assert.equal(host.camera.x, -16);
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
        const host = await load({ config: { centerFocusedColumn: "always" } });
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
        assert.equal(host.camera.x, 616);
        host.swipe(-600);
        assert.equal(host.camera.x, -16);
        assert.equal(host.focused, b);

        host.action("focus-column-last");
        host.swipe(-50, { x: -2000, y: 0 }, { x: -1000, y: 0 });
        assert.equal(host.camera.x, -16);
        assert.equal(host.focused, a);
    });

    test("with centered columns, the view snaps to the closest centered column", async () => {
        const host = await load({ config: { centerFocusedColumn: "always" } });
        const [, b] = openMany(host, 3);
        assert.equal(host.camera.x, 932);
        host.swipe(-700);
        assert.equal(host.focused, b);
        assert.deepEqual(host.screenRect(b), rect(332, 16, 616, 688));
    });

    test("small swipes snap back", async () => {
        const host = await load();
        const [, , c] = openMany(host, 3);
        host.swipe(-100);
        assert.equal(host.camera.x, 616);
        assert.equal(host.focused, c);
    });
});

describe("moving between workspaces", () => {
    test("whole columns move with their sizes", async () => {
        const from = await load();
        const [a, b] = openStacked(from);
        from.action("set-column-width", px(800));
        from.action("toggle-column-tabbed-display");
        const transfer = from.transferOut(a);
        assert.deepEqual(transfer.windows, [a, b]);
        for (const id of transfer.windows) {
            from.forget(id);
            from.send({ type: "remove", id, reason: "move" });
        }

        const to = await load();
        const c = to.open();
        const infos = transfer.windows.map(() => to.info(to.makeWindow({})));
        to.send({ type: "transfer-in", windows: infos, data: transfer.data, activate: true });
        const [na, nb] = infos.map((info) => info.id);
        assert.deepEqual(to.rect(c), rect(0, 16, 616, 688));
        assert.deepEqual(to.rect(na), rect(632, 16, 800, 688));
        assert.deepEqual(to.rect(nb), rect(632, 16, 800, 688));
        assert.equal(to.isVisible(na), false);
        assert.equal(to.focused, nb);
    });

    test("moved windows without data get columns of their own", async () => {
        const host = await load();
        const a = host.open();
        const win = host.makeWindow({});
        win.size = { w: 405, h: 688 };
        host.send({ type: "transfer-in", windows: [host.info(win)], data: null, activate: true });
        // 405 px matches the 1/3 preset, so the column keeps that.
        assert.deepEqual(host.rect(win.id), rect(632, 16, 405, 688));
        host.action("switch-preset-column-width");
        assert.equal(host.rect(win.id).w, 616);
        assert.deepEqual(host.rect(a), rect(0, 16, 616, 688));
    });
});

describe("reloading", () => {
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
        const before = { format: host.format(), camera: host.camera, focused: host.focused };

        host.reload();
        assert.deepEqual(
            { format: host.format(), camera: host.camera, focused: host.focused },
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
        assert.equal(host.focused, e);
        assert.deepEqual(
            [a, b, d].map((id) => host.ipc.get(id)),
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
        const c = host.open({ defaultWidth: { proportion: 1 / 3 } });
        const d = host.open();
        host.action("focus-column-left");
        const before = host.format();

        // `adopt` gets the windows where they were, and groups stacked ones into columns again.
        adoptFresh(host);
        assert.deepEqual(host.errors, []);
        assert.equal(host.format(), before);
        assert.equal(host.focused, c);
        assert.deepEqual(
            [a, b, c, d].map((id) => host.ipc.get(id)),
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
        win.size = { w: 600, h: 680 };
        host.send({ type: "change", windows: [host.info(win)] });
        assert.deepEqual(host.rect(a), rect(0, 16, 600, 680));
        assert.deepEqual(host.rect(b), rect(616, 16, 616, 688));
        // Asking for another size asks again.
        host.actionOn(a, "set-window-width", px(500));
        assert.deepEqual(host.rect(a), rect(0, 16, 500, 688));
    });

    test("the output size changes column sizes", async () => {
        const host = await load();
        const [a, b] = openMany(host, 2);
        host.resizeView({ size: { w: 1920, h: 1080 } });
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
        assert.deepEqual(host.newWindowSize({ rules: { defaultWidth: null } }), { w: 0, h: 688 });
        assert.deepEqual(host.newWindowSize({ rules: { defaultHeight: { proportion: 0.5 } } }), {
            w: 616,
            h: 336,
        });
        assert.deepEqual(
            host.newWindowSize({
                rules: { defaultWidth: { fixed: 500 } },
                borders: { w: 8, h: 8 },
            }),
            { w: 508, h: 688 },
        );
        // Fixed-size windows get their size.
        assert.deepEqual(
            host.newWindowSize({ minSize: { w: 300, h: 200 }, maxSize: { w: 300, h: 200 } }),
            { w: 300, h: 200 },
        );
    });

    test("IPC positions are 1-based column and tile", async () => {
        const host = await load();
        const [a, b] = openStacked(host);
        const c = host.open();
        assert.deepEqual(host.ipc.get(a), [1, 1]);
        assert.deepEqual(host.ipc.get(b), [1, 2]);
        assert.deepEqual(host.ipc.get(c), [2, 1]);
    });

    test("unknown actions do nothing", async () => {
        const host = await load();
        host.open();
        assert.equal(host.action("not-an-action"), false);
        assert.equal(host.action("set-column-width", "bogus"), false);
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
                scale: 1.5,
                workingArea: { x: 0, y: 25, w: 1280, h: 695 },
                config: {
                    centerFocusedColumn: "always",
                    struts: { left: 20.5, right: 10, top: 5, bottom: 0 },
                    tabIndicator: {
                        off: false,
                        position: "top",
                        width: 4,
                        gap: 5,
                        placeWithinColumn: true,
                        hideWhenSingleTab: true,
                    },
                },
            },
        ],
        [
            "on-overflow, single column centered",
            {
                config: {
                    centerFocusedColumn: "on-overflow",
                    alwaysCenterSingleColumn: true,
                    gaps: 7,
                },
            },
        ],
    ]) {
        test(`layouts stay consistent: ${name}`, async () => {
            const host = await load(opts);
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
                        host.swipe(next(1600) - 800);
                    } else if (op === 12) {
                        const id = pick();
                        host.drag(
                            id,
                            { x: host.camera.x + next(1280), y: next(720) },
                            host.rect(id),
                        );
                    } else {
                        host.reload();
                    }
                    assert.deepEqual(host.errors, []);
                    assert.deepEqual(host.logs, [], `seed ${seed} step ${step}`);
                    for (const id of ids()) assert.ok(host.rect(id), `window ${id} has a rect`);
                    if (host.focused !== null) {
                        assert.equal(host.isVisible(host.focused), true);
                        // The view always scrolls to show the focused window, or its left
                        // edge if it's wider than the working area.
                        const screen = host.screenRect(host.focused);
                        const { left, right } = host.config.struts;
                        const fits = screen.w <= host.view.workingArea.w - left - right;
                        assert.ok(
                            screen.x >= -1 &&
                                (!fits || screen.x + screen.w <= host.view.size.w + 1),
                            `seed ${seed} step ${step}: focused window at ${JSON.stringify(screen)}`,
                        );
                    }
                }
                for (const id of ids()) host.close(id);
            }
        });
    }
});
