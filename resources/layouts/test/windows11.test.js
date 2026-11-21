// Tests for the windows11 engine, `../windows11.js`. The output is 1280 × 720 with 16 px gaps, so
// the halves are 616 px wide with a gap around each, and the quarters 616 × 336.

import assert from "node:assert/strict";
import { test } from "node:test";

import { TestHost } from "./host.js";

const load = (opts) => TestHost.load("../windows11.js", opts);
const rect = (x, y, w, h) => ({ x, y, w, h });

const LEFT = rect(16, 16, 616, 688);
const RIGHT = rect(648, 16, 616, 688);
const TOP_LEFT = rect(16, 16, 616, 336);
const TOP_RIGHT = rect(648, 16, 616, 336);
const BOTTOM_RIGHT = rect(648, 368, 616, 336);
const SIZE = { w: 640, h: 400 };
/** Where a 640 × 400 window opens, centred. */
const CENTRED = rect(320, 160, 640, 400);

/** What the engine shows with the keyboard: the switcher, Snap Assist or the layouts picker. */
const showing = (h) => h.exposedState().showing;
const stateOf = (h, id) => h.exposedState().windows.find((win) => win.id === id);
const parentOf = (h, id) => h.tree().node(`w${id}`).parent;
/** The copy node that shows window `id`, if there's one. */
const copyOf = (h, id) =>
    h.tree().order().find((node) => {
        const { kind, props } = h.tree().node(node);
        return kind === "copy" && props.window === id;
    });

/**
 * Drags window `id` over the active workspace through `positions`, without dropping it, held
 * `offset` from its top left. The drag starts the first time.
 */
function dragOver(h, id, positions, offset = { x: 10, y: 10 }) {
    h.dragging ??= new Set();
    if (!h.dragging.has(id)) {
        h.dragging.add(id);
        const at = h.rect(id);
        const pointer = { x: at.x + offset.x, y: at.y + offset.y };
        h.send({ type: "dragstart", id, offset, pointer });
    }
    const workspace = h.activeWorkspace().id;
    for (const pointer of positions) h.send({ type: "dragmove", id, workspace, pointer });
}

/** Ends the drag of window `id` that `dragOver` started: dropped, or cancelled. */
function dragEnd(h, id, dropped = true) {
    h.dragging.delete(id);
    h.send({ type: "dragend", id, dropped });
}

/** How window `id`'s node is drawn moved and scaled. */
const transformOf = (h, id) => h.tree().node(`w${id}`).props.transform ?? { scale: 1, x: 0, y: 0 };

/** The boxes drawn on the active workspace, as their props. */
function boxes(h) {
    const tree = h.tree();
    return tree
        .order()
        .filter((id) => tree.node(id).kind === "box")
        .map((id) => tree.node(id).props);
}

test("new windows open at their size, centred, a step down and right of one already there", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    const c = h.open({ size: SIZE });
    assert.deepEqual(h.rect(a), CENTRED);
    assert.deepEqual(h.rect(b), rect(352, 192, 640, 400));
    assert.deepEqual(h.rect(c), rect(384, 224, 640, 400));
    assert.equal(h.focused(), c);
    assert.deepEqual(h.stacking(), [a, b, c]);

    // Where the next step leaves the screen, it starts again from the top left.
    const tall = await load();
    tall.open({ size: { w: 800, h: 600 } });
    tall.open({ size: { w: 800, h: 600 } });
    assert.deepEqual(tall.rect(tall.open({ size: { w: 800, h: 600 } })), rect(16, 16, 800, 600));

    // A window that hasn't said its size gets one, and one too large for the screen fits in it.
    const d = h.open();
    assert.deepEqual(h.rect(d), rect(256, 108, 768, 504));
    const e = h.open({ size: { w: 3000, h: 2000 } });
    assert.deepEqual(h.rect(e), rect(16, 16, 1248, 688));

    // The new window takes the size it asked for, so it's answered to let the window decide.
    assert.deepEqual(h.newWindowSize(), { w: 0, h: 0 });
});

test("dialogs open centred over their parent", async () => {
    const h = await load();
    const parent = h.open({ size: SIZE });
    const dialog = h.open({ size: { w: 400, h: 300 }, nextTo: parent });
    assert.deepEqual(h.rect(dialog), rect(440, 210, 400, 300));

    // The dialog's request to float centres it again, at the size it asked for.
    h.send({ type: "floatrequest", id: dialog, reason: "dialog", parent, size: { w: 300, h: 200 } });
    assert.deepEqual(h.rect(dialog), rect(490, 260, 300, 200));

    // Other requests to float change nothing: every window is free.
    h.send({ type: "floatrequest", id: dialog, reason: "action", parent: null, size: null });
    assert.deepEqual(h.rect(dialog), rect(490, 260, 300, 200));
});

test("clicking a window focuses and raises it", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    h.activate(a);
    assert.equal(h.focused(), a);
    assert.deepEqual(h.stacking(), [b, a]);
});

test("a dragged window follows the pointer, above the others, onto the workspace under it", async () => {
    const h = await load({ monitors: [{ name: "DP-1" }, { name: "DP-2" }] });
    const right = h.workspace(0, "DP-2").id;
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    h.activate(a);

    dragOver(h, b, [{ x: 400, y: 300 }]);
    assert.deepEqual(h.rect(b), rect(390, 290, 640, 400));
    assert.equal(h.focused(), b);
    assert.equal(h.stacking().at(-1), b);
    dragOver(h, b, [{ x: 500, y: 350 }]);
    assert.deepEqual(h.rect(b), rect(490, 340, 640, 400));

    // Onto the other monitor, where it's dropped and has the focus.
    const ws = h.workspace(0, "DP-1").id;
    h.send({ type: "dragmove", id: b, workspace: right, pointer: { x: 100, y: 100 } });
    assert.equal(h.windows.get(b).workspace, right);
    assert.deepEqual(h.rect(b), rect(90, 90, 640, 400));
    dragEnd(h, b);
    assert.deepEqual(h.rect(b), rect(90, 90, 640, 400));
    assert.deepEqual(stateOf(h, b).free, rect(90, 90, 640, 400));
    assert.equal(h.focusedMonitor, "DP-2");
    assert.equal(h.focused(), b);
    assert.deepEqual(h.stacking(ws), [a]);
});

test("dropping a window at an edge or a corner snaps it, and at the top maximizes it", async () => {
    const h = await load({ settings: { "snap-assist": false } });
    const a = h.open({ size: SIZE });
    const ws = h.activeWorkspace().id;

    h.drag(a, ws, [{ x: 400, y: 300 }, { x: 5, y: 360 }]);
    assert.deepEqual(h.rect(a), LEFT);
    h.drag(a, ws, [{ x: 1275, y: 360 }]);
    assert.deepEqual(h.rect(a), RIGHT);
    h.drag(a, ws, [{ x: 5, y: 5 }]);
    assert.deepEqual(h.rect(a), TOP_LEFT);
    h.drag(a, ws, [{ x: 1275, y: 715 }]);
    assert.deepEqual(h.rect(a), BOTTOM_RIGHT);
    // Along the top edge, near a corner, is that corner's quarter too.
    h.drag(a, ws, [{ x: 1200, y: 5 }]);
    assert.deepEqual(h.rect(a), TOP_RIGHT);

    h.drag(a, ws, [{ x: 640, y: 5 }]);
    assert.deepEqual(h.rect(a), rect(0, 0, 1280, 720));
    assert.equal(h.mode(a), "maximized");
    assert.equal(h.focused(), a);
});

test("a window dragged away from its zone or maximized gets its own size back", async () => {
    const h = await load({ settings: { "snap-assist": false } });
    const a = h.open({ size: SIZE });
    const ws = h.activeWorkspace().id;

    h.drag(a, ws, [{ x: 5, y: 360 }]);
    assert.deepEqual(h.rect(a), LEFT);
    // As soon as it moves, it's under the pointer where it was held, in proportion: here the
    // middle of its top part.
    dragOver(h, a, [], { x: 308, y: 43 });
    assert.equal(stateOf(h, a).zone, null);
    assert.deepEqual(h.rect(a), rect(4, 34, 640, 400));
    dragOver(h, a, [{ x: 640, y: 360 }]);
    dragEnd(h, a);
    assert.deepEqual(h.rect(a), rect(320, 335, 640, 400));

    h.drag(a, ws, [{ x: 640, y: 5 }]);
    assert.equal(h.mode(a), "maximized");
    h.drag(a, ws, [{ x: 500, y: 200 }], true, { x: 640, y: 36 });
    assert.equal(h.mode(a), "normal");
    assert.deepEqual(h.rect(a), rect(180, 180, 640, 400));

    // A free window stays where it's dropped.
    h.drag(a, ws, [{ x: 300, y: 100 }]);
    assert.deepEqual(h.rect(a), rect(290, 90, 640, 400));
});

test("snapping by a drop keeps the place it had, which the keyboard restores", async () => {
    const h = await load({ settings: { "snap-assist": false } });
    const a = h.open({ size: SIZE });
    h.drag(a, h.activeWorkspace().id, [{ x: 5, y: 360 }]);
    h.action("snap-right");
    assert.deepEqual(h.rect(a), CENTRED);
});

test("a preview shows where a dragged window would snap, and fades when it wouldn't", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const previews = () => boxes(h).filter((box) => box.background === "#c8c8c84d");

    dragOver(h, a, [{ x: 400, y: 300 }]);
    assert.equal(previews().length, 0);
    dragOver(h, a, [{ x: 5, y: 360 }]);
    const [preview] = previews();
    assert.deepEqual(preview.rect, LEFT);
    assert.equal(preview.opacity, 1);
    // It grows from where the dragged window is, which stays above it.
    assert.deepEqual(preview.enter, { opacity: 0, rect: rect(-5, 350, 640, 400) });
    const tree = h.tree();
    assert.equal(tree.node(tree.rootId).children.at(-1), `w${a}`);

    dragOver(h, a, [{ x: 400, y: 300 }]);
    assert.equal(previews()[0].opacity, 0);
    h.advanceTime(500);
    assert.equal(previews().length, 0);

    // And at the end of the drag.
    dragOver(h, a, [{ x: 5, y: 360 }]);
    dragEnd(h, a);
    assert.equal(previews()[0].opacity, 0);
    h.advanceTime(500);
    assert.equal(previews().length, 0);
});

test("a cancelled drag leaves the window where it was carried, without snapping", async () => {
    const h = await load({ settings: { "snap-assist": false } });
    const a = h.open({ size: SIZE });
    const ws = h.activeWorkspace().id;
    const previews = () => boxes(h).filter((box) => box.background === "#c8c8c84d");
    h.drag(a, ws, [{ x: 5, y: 360 }]);
    assert.deepEqual(h.rect(a), LEFT);

    // Held in the middle of its top, it has its own size from the start.
    h.drag(a, ws, [{ x: 5, y: 360 }], false, { x: 308, y: 10 });
    assert.deepEqual(h.rect(a), rect(-315, 354, 640, 400));
    assert.equal(stateOf(h, a).zone, null);
    assert.equal(h.mode(a), "normal");
    h.advanceTime(500);
    assert.equal(previews().length, 0);
});

test("a window closed while it's dragged takes its preview with it", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    const previews = () => boxes(h).filter((box) => box.background === "#c8c8c84d");
    dragOver(h, b, [{ x: 5, y: 360 }]);
    assert.equal(previews().length, 1);
    h.close(b);
    h.advanceTime(500);
    assert.equal(previews().length, 0);
    assert.equal(h.focused(), a);
    assert.deepEqual(h.errors, []);
});

test("with edge-snap off, a window dropped at an edge stays where it's dropped", async () => {
    const h = await load({ settings: { "edge-snap": false } });
    const a = h.open({ size: SIZE });
    h.drag(a, h.activeWorkspace().id, [{ x: 5, y: 360 }]);
    assert.deepEqual(h.rect(a), rect(-5, 350, 640, 400));
    assert.equal(stateOf(h, a).zone, null);
});

test("snap-left and snap-right snap to a half, and from the other half restore", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });

    h.action("snap-left");
    assert.deepEqual(h.rect(a), LEFT);
    // There's no monitor on the left to go on to.
    h.action("snap-left");
    assert.deepEqual(h.rect(a), LEFT);
    h.action("snap-right");
    assert.deepEqual(h.rect(a), CENTRED);
    h.action("snap-right");
    assert.deepEqual(h.rect(a), RIGHT);
    h.action("snap-left");
    assert.deepEqual(h.rect(a), CENTRED);

    // A maximized window snaps too.
    h.requestMode(a, "maximized");
    h.action("snap-left");
    assert.equal(h.mode(a), "normal");
    assert.deepEqual(h.rect(a), LEFT);
});

test("snap-up and snap-down go between halves, quarters, maximized and minimized", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });

    h.action("snap-right");
    h.action("snap-up");
    assert.deepEqual(h.rect(a), TOP_RIGHT);
    // A quarter goes to the one beside it.
    h.action("snap-left");
    assert.deepEqual(h.rect(a), TOP_LEFT);
    h.action("snap-right");
    assert.deepEqual(h.rect(a), TOP_RIGHT);
    h.action("snap-down");
    assert.deepEqual(h.rect(a), RIGHT);
    h.action("snap-down");
    assert.deepEqual(h.rect(a), BOTTOM_RIGHT);
    h.action("snap-up");
    assert.deepEqual(h.rect(a), RIGHT);
    h.action("snap-up");
    h.action("snap-up");
    assert.equal(h.mode(a), "maximized");

    // Down from a quarter at the bottom minimizes.
    h.action("snap-down");
    assert.equal(h.mode(a), "normal");
    assert.deepEqual(h.rect(a), TOP_RIGHT);
    h.action("snap-down");
    h.action("snap-down");
    assert.deepEqual(h.rect(a), BOTTOM_RIGHT);
    h.action("snap-down");
    assert.equal(h.isVisible(a), false);
});

test("from its own size, up maximizes, and down restores and then minimizes", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    h.action("snap-up");
    assert.equal(h.mode(a), "maximized");
    assert.deepEqual(h.rect(a), rect(0, 0, 1280, 720));
    h.action("snap-down");
    assert.equal(h.mode(a), "normal");
    assert.deepEqual(h.rect(a), CENTRED);
    h.action("snap-down");
    assert.equal(h.isVisible(a), false);
    assert.equal(stateOf(h, a).minimized, true);
});

test("snapped windows side by side resize together across their shared edge", async () => {
    const h = await load({ settings: { "snap-assist": false } });
    const a = h.open({ size: SIZE });
    h.action("snap-left");
    const b = h.open({ size: SIZE });
    h.action("snap-right");

    assert.ok(h.resize(a, { right: true }, { x: 100, y: 0 }));
    assert.deepEqual(h.rect(a), rect(16, 16, 716, 688));
    assert.deepEqual(h.rect(b), rect(748, 16, 516, 688));

    // Back the other way, from the other window, which can't make either too small.
    h.resize(b, { left: true }, { x: 1000, y: 0 });
    assert.equal(h.rect(b).w, 160 - 16);
    assert.equal(h.rect(a).x + h.rect(a).w + 16, h.rect(b).x);
});

test("an edge shared by a half and two quarters moves for all three", async () => {
    const h = await load({ settings: { "snap-assist": false } });
    const a = h.open({ size: SIZE });
    h.action("snap-left");
    const b = h.open({ size: SIZE });
    h.action("snap-right");
    h.action("snap-up");
    const c = h.open({ size: SIZE });
    h.action("snap-right");
    h.action("snap-down");

    h.resize(b, { left: true }, { x: -100, y: 0 });
    assert.deepEqual(h.rect(a), rect(16, 16, 516, 688));
    assert.deepEqual(h.rect(b), rect(548, 16, 716, 336));
    assert.deepEqual(h.rect(c), rect(548, 368, 716, 336));

    // The edge between the quarters is theirs alone.
    h.resize(b, { bottom: true }, { x: 0, y: 50 });
    assert.deepEqual(h.rect(b), rect(548, 16, 716, 386));
    assert.deepEqual(h.rect(c), rect(548, 418, 716, 286));
    assert.deepEqual(h.rect(a), rect(16, 16, 516, 688));

    // An edge on the screen's edge moves only its window.
    h.resize(a, { left: true }, { x: 50, y: 0 });
    assert.deepEqual(h.rect(a), rect(66, 16, 466, 688));
    assert.deepEqual(h.rect(c), rect(548, 418, 716, 286));
});

test("a free window resizes alone, and a maximized one can't be resized", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    h.resize(a, { left: true, top: true }, { x: -40, y: 20 });
    assert.deepEqual(h.rect(a), rect(280, 180, 680, 380));
    assert.deepEqual(stateOf(h, a).free, rect(280, 180, 680, 380));

    h.requestMode(a, "maximized");
    assert.equal(h.resize(a, { right: true }, { x: 10, y: 0 }), false);
});

test("maximized fills the working area, fullscreen the monitor, and leaving goes back", async () => {
    const workingArea = rect(0, 32, 1280, 688);
    const h = await load({ monitors: [{ name: "DP-1", workingArea }] });
    const a = h.open({ size: SIZE });
    assert.deepEqual(h.rect(a), rect(320, 176, 640, 400));

    h.requestMode(a, "maximized");
    assert.equal(h.mode(a), "maximized");
    assert.deepEqual(h.rect(a), workingArea);
    h.requestMode(a, "normal", { maximized: false });
    assert.deepEqual(h.rect(a), rect(320, 176, 640, 400));

    h.requestMode(a, "fullscreen");
    assert.deepEqual(h.rect(a), rect(0, 0, 1280, 720));
    h.requestMode(a, "normal", { fullscreen: false });
    assert.deepEqual(h.rect(a), rect(320, 176, 640, 400));

    // A snapped window goes back to its zone.
    h.action("snap-left");
    h.requestMode(a, "maximized");
    h.requestMode(a, "normal", { maximized: false });
    assert.deepEqual(h.rect(a), rect(16, 48, 616, 656));
});

test("minimizing hides a window and focuses the last one used, and activating brings it back", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    const c = h.open({ size: SIZE });
    h.activate(a);
    h.activate(c);

    h.send({ type: "minimizerequest", id: c });
    assert.equal(h.isVisible(c), false);
    assert.equal(h.focused(), a);
    h.action("minimize");
    assert.equal(h.isVisible(a), false);
    assert.equal(h.focused(), b);

    // A taskbar or the app asks for it.
    h.activate(c);
    assert.equal(h.isVisible(c), true);
    assert.equal(h.focused(), c);
    assert.equal(stateOf(h, c).minimized, false);
    assert.deepEqual(h.rect(c), rect(384, 224, 640, 400));
    assert.equal(h.stacking().at(-1), c);
});

test("show-desktop minimizes every window, and brings them back with the focus", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    const c = h.open({ size: SIZE });
    h.activate(b);
    h.send({ type: "minimizerequest", id: c });

    h.action("show-desktop");
    assert.deepEqual([a, b, c].map((id) => h.isVisible(id)), [false, false, false]);
    h.action("show-desktop");
    // The window minimized before stays minimized.
    assert.deepEqual([a, b, c].map((id) => h.isVisible(id)), [true, true, false]);
    assert.equal(h.focused(), b);
    assert.deepEqual(h.stacking(), [a, c, b]);
});

test("Alt+Tab goes through the windows, most recently used first, and letting go of Alt picks", async () => {
    const h = await load();
    const a = h.open({ size: SIZE, title: "A" });
    const b = h.open({ size: SIZE, title: "B" });
    const c = h.open({ size: SIZE, title: "C" });
    h.activate(a);

    h.keyDown("Alt_L");
    h.action("switch-windows");
    assert.deepEqual(showing(h), { kind: "switcher", workspace: 1, windows: [a, c, b], selected: 1 });
    assert.equal(h.keyboard.grabbed, true);
    // The windows show as copies, with their titles, and stay where they are.
    for (const id of [a, b, c]) {
        assert.notEqual(copyOf(h, id), undefined);
        assert.equal(parentOf(h, id), h.tree().rootId);
        assert.equal(h.tree().node(`w${id}`).props.input ?? "app", "app");
    }
    assert.ok(h.tree().order().some((id) => h.tree().node(id).props.text === "C"));

    assert.equal(h.key("Tab"), "engine");
    assert.equal(showing(h).selected, 2);
    h.key("ISO_Left_Tab", { shift: true });
    h.key("Tab");
    h.key("Tab");
    assert.equal(showing(h).selected, 0);
    h.key("Right");
    h.keyUp("Alt_L");

    assert.equal(showing(h), null);
    assert.equal(h.focused(), c);
    assert.equal(h.keyboard.grabbed, false);
    // The switcher left the windows as they were.
    assert.equal(parentOf(h, c), h.tree().rootId);
    assert.equal(h.tree().node(`w${c}`).props.input ?? "app", "app");
    assert.deepEqual(h.tree().node(`w${c}`).props.transform ?? { scale: 1, x: 0, y: 0 }, {
        scale: 1,
        x: 0,
        y: 0,
    });
    assert.deepEqual(h.stacking(), [b, a, c]);
    assert.deepEqual(h.exposedState().recent, [c, a, b]);
});

test("a quick Alt+Tab goes back to the window used before", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    h.open({ size: SIZE });
    h.keyDown("Alt_L");
    h.action("switch-windows");
    h.keyUp("Alt_L");
    assert.equal(h.focused(), a);
});

test("Alt+Tab shows minimized windows, and picking one brings it back", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    h.send({ type: "minimizerequest", id: b });
    assert.equal(h.focused(), a);

    h.keyDown("Alt_L");
    h.action("switch-windows");
    assert.deepEqual(showing(h).windows, [a, b]);
    // A copy shows it, and it stays minimized until it's picked.
    assert.notEqual(copyOf(h, b), undefined);
    assert.equal(h.isVisible(b), false);
    h.keyUp("Alt_L");
    assert.equal(h.focused(), b);
    assert.equal(h.isVisible(b), true);
    assert.equal(stateOf(h, b).minimized, false);
});

test("Escape closes the switcher without switching, and a minimized window stays so", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    const c = h.open({ size: SIZE });
    h.send({ type: "minimizerequest", id: a });

    h.keyDown("Alt_L");
    h.action("switch-windows-back");
    assert.equal(showing(h).selected, 2);
    h.key("Escape");
    assert.equal(showing(h), null);
    assert.equal(h.keyboard.grabbed, false);
    h.keyUp("Alt_L");
    assert.equal(h.focused(), c);
    assert.equal(h.isVisible(a), false);
    assert.deepEqual([a, b, c].map((id) => parentOf(h, id)), [1, 2, 3].map(() => "r1"));
});

test("clicking a thumbnail in the switcher picks it, and clicking elsewhere closes it", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });

    h.keyDown("Alt_L");
    h.action("switch-windows");
    h.pressOutsideNodes();
    assert.equal(showing(h), null);
    assert.equal(h.focused(), b);

    h.action("switch-windows");
    h.pointer(copyOf(h, a), "click");
    assert.equal(h.focused(), a);
    assert.equal(showing(h), null);
    h.keyUp("Alt_L");
    assert.equal(h.focused(), a);
});

test("after a window snaps, Snap Assist offers the others for the space left", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    const c = h.open({ size: SIZE });

    h.action("snap-left");
    assert.deepEqual(showing(h), { kind: "assist", workspace: 1, windows: [b, a], selected: 0 });
    assert.equal(h.keyboard.grabbed, true);
    // The thumbnails fill the right half.
    const panel = boxes(h).find((box) => box.background === "#202020e0");
    assert.deepEqual(panel.rect, RIGHT);

    h.key("Right");
    h.key("Return");
    assert.deepEqual(h.rect(a), RIGHT);
    assert.deepEqual(h.rect(c), LEFT);
    assert.deepEqual(h.rect(b), rect(352, 192, 640, 400));
    assert.equal(h.focused(), a);
    // The screen is full, so it's done.
    assert.equal(showing(h), null);
    assert.equal(h.keyboard.grabbed, false);
});

test("Snap Assist goes on until the screen is full, and Escape dismisses it", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    const c = h.open({ size: SIZE });

    h.action("snap-left");
    h.action("snap-up");
    // Next to a quarter, the right half is free.
    assert.deepEqual(showing(h).windows, [b, a]);
    h.pointer(`w${b}`, "click");
    assert.deepEqual(h.rect(b), RIGHT);
    // Then the quarter below.
    assert.deepEqual(showing(h).windows, [a]);
    h.key("Escape");
    assert.equal(showing(h), null);
    assert.deepEqual(h.rect(a), CENTRED);
    assert.deepEqual(h.rect(c), TOP_LEFT);
});

test("Snap Assist follows a drop, with the pointer, and can be turned off", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    h.drag(b, h.activeWorkspace().id, [{ x: 1275, y: 360 }]);
    assert.deepEqual(showing(h).windows, [a]);
    // A drop isn't a key or a click, so the keyboard stays with the apps.
    assert.equal(h.keyboard.grabbed, false);
    h.pointer(`w${a}`, "click");
    assert.deepEqual(h.rect(a), LEFT);

    const off = await load({ settings: { "snap-assist": false } });
    off.open({ size: SIZE });
    off.open({ size: SIZE });
    off.action("snap-left");
    assert.equal(showing(off), null);
});

test("show-layouts snaps the focused window into a zone, then offers the rest", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    const c = h.open({ size: SIZE });

    h.action("show-layouts");
    assert.equal(showing(h).kind, "layouts");
    assert.equal(h.keyboard.grabbed, true);
    // The zones of every layout, one after the other: two halves, two thirds and one, three
    // columns, then quarters. The first of the three columns:
    for (let i = 0; i < 4; i++) h.key("Right");
    h.key("Return");
    assert.deepEqual(h.rect(c), rect(16, 16, 405, 688));
    assert.deepEqual(showing(h).windows, [b, a]);
    h.key("Return");
    assert.deepEqual(h.rect(b), rect(437, 16, 406, 688));
    assert.deepEqual(showing(h).windows, [a]);
    h.key("Return");
    assert.deepEqual(h.rect(a), rect(859, 16, 405, 688));
    assert.equal(showing(h), null);
    assert.equal(h.keyboard.grabbed, false);
});

test("show-layouts closes with Escape, or a click outside it", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    h.action("show-layouts");
    h.key("Escape");
    assert.equal(showing(h), null);
    assert.deepEqual(h.rect(a), CENTRED);

    h.action("show-layouts");
    h.pressOutsideNodes();
    assert.equal(showing(h), null);
    assert.equal(h.keyboard.grabbed, false);
});

test("snapping toward a monitor from that half moves the window there, mirrored", async () => {
    const h = await load({ monitors: [{ name: "DP-1" }, { name: "DP-2" }] });
    const left = h.workspace(0, "DP-1").id;
    const right = h.workspace(0, "DP-2").id;
    const a = h.open({ size: SIZE, workspace: right });
    h.actionOn(a, "snap-left");
    assert.deepEqual(h.rect(a), LEFT);
    h.actionOn(a, "snap-left");
    assert.equal(h.windows.get(a).workspace, left);
    assert.deepEqual(h.rect(a), RIGHT);
    assert.equal(h.focusedMonitor, "DP-1");
    assert.equal(h.focused(), a);
});

test("windows moved to another monitor keep their place there", async () => {
    const h = await load({ monitors: [{ name: "DP-1" }, { name: "DP-2" }] });
    const left = h.workspace(0, "DP-1").id;
    const right = h.workspace(0, "DP-2").id;
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });

    h.actionOn(a, "move-to-monitor-right");
    assert.equal(h.windows.get(a).workspace, right);
    assert.deepEqual(h.rect(a), CENTRED);
    assert.equal(h.focusedMonitor, "DP-2");

    // bento's own move keeps a snapped window in its zone.
    h.actionOn(b, "snap-left");
    h.move(b, right, true);
    assert.deepEqual(h.rect(b), LEFT);
    assert.equal(h.windows.get(b).workspace, right);
    assert.equal(h.workspace(0, "DP-1").id, left);
});

test("closing the focused window focuses the one used before it", async () => {
    const h = await load();
    const a = h.open({ size: SIZE });
    const b = h.open({ size: SIZE });
    h.open({ size: SIZE });
    h.activate(a);
    h.activate(b);
    h.close(b);
    assert.equal(h.focused(), a);
});

test("reloading keeps zones, sizes, minimized windows and the order used", async () => {
    const h = await load({ settings: { "snap-assist": false } });
    const a = h.open({ size: SIZE });
    h.action("snap-left");
    const b = h.open({ size: SIZE });
    h.send({ type: "minimizerequest", id: b });
    const before = h.exposedState();

    h.reload();
    assert.deepEqual(h.exposedState().windows, before.windows);
    assert.deepEqual(h.exposedState().recent, before.recent);
    assert.equal(h.isVisible(b), false);
    h.action("snap-right");
    assert.deepEqual(h.rect(a), CENTRED);
    assert.deepEqual(h.errors, []);
});

test("what's drawn stays above windows that open and get raised", async () => {
    const h = await load();
    h.open({ size: SIZE });
    h.open({ size: SIZE });
    h.action("snap-left");
    const tree = h.tree();
    const last = tree.node(tree.rootId).children.at(-1);
    assert.equal(tree.node(last).kind, "box");
});

// Groups, shown as tabs. A window keeps its place in a group, with the tab strip right above it.
const groupOf = (h, id) => h.groups.find((group) => group.windows.includes(id)) ?? null;
/** The node drawn with `text`, like a tab's title or a button's glyph. */
const textNode = (h, text) => {
    const tree = h.tree();
    return tree.order().find((node) => tree.node(node).props.text === text);
};
/** The tab drawn for the window titled `title`. */
const tabOf = (h, title) => h.tree().node(textNode(h, title)).parent;
/** Above the window opened first, where its tab strip would go. */
const ABOVE = { x: 600, y: 140 };
/** Where the pointer holds a group's active window when it presses its strip: above its top. */
const ON_STRIP = { x: 100, y: -18 };
/** The tab strip of the group that has the window titled `title`. */
const stripOf = (h, title) => h.tree().node(tabOf(h, title)).parent;

test("a window dropped right above another joins it as a tab", async () => {
    const h = await load();
    const a = h.open({ size: SIZE, title: "A" });
    const b = h.open({ size: SIZE, title: "B" });
    const ws = h.activeWorkspace().id;

    // On its top part, a window is just dropped there.
    h.drag(b, ws, [{ x: 600, y: 170 }]);
    assert.equal(groupOf(h, b), null);

    // Above it, the two are a group, the dropped one active, in the other's place.
    h.drag(b, ws, [{ x: 400, y: 300 }, ABOVE]);
    const group = groupOf(h, b);
    assert.deepEqual(group.windows, [a, b]);
    assert.equal(group.active, b);
    assert.deepEqual(h.rect(b), CENTRED);
    assert.deepEqual(h.rect(a), CENTRED);
    assert.equal(h.windows.get(a).visible, false);
    assert.equal(h.windows.get(b).visible, true);
    assert.equal(h.exposedState().groups[0].tabs, true);
});

test("over where it would join a window as a tab, a dragged window is a thumbnail", async () => {
    const h = await load();
    const a = h.open({ size: SIZE, title: "A" });
    const b = h.open({ size: SIZE, title: "B" });
    const previews = () => boxes(h).filter((box) => box.background === "#c8c8c84d");

    // Centred on the pointer, as large as fits 240 × 160, and growing back when it leaves.
    dragOver(h, b, [ABOVE]);
    assert.deepEqual(h.rect(b), rect(590, 130, 640, 400));
    assert.deepEqual(transformOf(h, b), { scale: 0.375, x: -310, y: -190 });
    assert.deepEqual(h.tree().node(`w${b}`).props.transitions.transform, {
        duration: 180,
        curve: "ease-out-cubic",
    });
    // The preview shows the strip it would join.
    assert.deepEqual(previews()[0].rect, rect(320, 124, 640, 36));
    assert.equal(previews()[0].opacity, 1);

    dragOver(h, b, [{ x: 400, y: 400 }]);
    assert.deepEqual(transformOf(h, b), { scale: 1, x: 0, y: 0 });
    assert.deepEqual(h.rect(b), rect(390, 390, 640, 400));
    assert.equal(previews()[0].opacity, 0);

    // Dropped as a thumbnail, it joins at its full size.
    dragOver(h, b, [ABOVE]);
    dragEnd(h, b);
    assert.deepEqual(groupOf(h, b).windows, [a, b]);
    assert.deepEqual(transformOf(h, b), { scale: 1, x: 0, y: 0 });
    assert.deepEqual(h.rect(b), CENTRED);
});

test("pressing a tab brings its window to the front, and dragging it pulls it out", async () => {
    const h = await load();
    const a = h.open({ size: SIZE, title: "A" });
    const b = h.open({ size: SIZE, title: "B" });
    const ws = h.activeWorkspace().id;
    h.drag(b, ws, [ABOVE]);

    h.pointer(tabOf(h, "A"), "pointerdown");
    assert.equal(groupOf(h, a).active, a);
    assert.equal(h.windows.get(a).visible, true);
    assert.equal(h.windows.get(b).visible, false);
    assert.deepEqual(h.takeGrab(), { kind: "move", window: a });

    // Pulled out by its tab as soon as it moves, it's on its own, its top where the strip's was,
    // and the other shows again.
    dragOver(h, a, [], { x: 30, y: -18 });
    assert.equal(groupOf(h, a), null);
    assert.equal(h.windows.get(b).visible, true);
    assert.deepEqual(h.rect(b), CENTRED);
    assert.deepEqual(h.rect(a), rect(320, 124, 640, 400));
    assert.deepEqual(h.exposedState().groups, []);
    // Then it's dropped where the pointer takes it.
    dragOver(h, a, [{ x: 900, y: 650 }]);
    dragEnd(h, a);
    assert.equal(groupOf(h, a), null);
    assert.deepEqual(h.rect(a), rect(870, 632, 640, 400));
    assert.deepEqual(h.rect(b), CENTRED);
});

test("a group moves as one, by its strip or its active window's own title bar", async () => {
    const h = await load();
    const a = h.open({ size: SIZE, title: "A" });
    const b = h.open({ size: SIZE, title: "B" });
    const ws = h.activeWorkspace().id;
    h.drag(b, ws, [ABOVE]);

    // By the window's own title bar.
    h.drag(b, ws, [{ x: 640, y: 400 }]);
    let group = groupOf(h, b);
    assert.deepEqual(new Set(group.windows), new Set([a, b]));
    assert.equal(group.active, b);
    assert.equal(h.windows.get(a).visible, false);
    assert.deepEqual(h.rect(a), h.rect(b));
    assert.notDeepEqual(h.rect(b), CENTRED);

    // By the strip: it moves the active window, and the group follows it, snapping and all.
    h.pointer(stripOf(h, "A"), "pointerdown");
    assert.deepEqual(h.takeGrab(), { kind: "move", window: b });
    h.drag(b, ws, [{ x: 5, y: 360 }], true, ON_STRIP);
    group = groupOf(h, b);
    assert.deepEqual(new Set(group.windows), new Set([a, b]));
    assert.deepEqual(h.rect(b), rect(16, 52, 616, 652));
    assert.deepEqual(h.rect(a), h.rect(b));
});

test("a group follows its strip while it's dragged, as a thumbnail too", async () => {
    const h = await load();
    const a = h.open({ size: SIZE, title: "A" });
    const b = h.open({ size: SIZE, title: "B" });
    const ws = h.activeWorkspace().id;
    h.drag(b, ws, [ABOVE]);
    const c = h.open({ size: SIZE, title: "C" });
    assert.deepEqual(h.rect(c), rect(352, 192, 640, 400));

    h.pointer(stripOf(h, "A"), "pointerdown");
    assert.deepEqual(h.takeGrab(), { kind: "move", window: b });
    dragOver(h, b, [{ x: 700, y: 400 }], ON_STRIP);
    // The active window under the pointer, the strip right above it, the other tab hidden.
    assert.deepEqual(h.rect(b), rect(600, 418, 640, 400));
    assert.deepEqual(h.tree().node(stripOf(h, "A")).props.rect, rect(600, 382, 640, 36));
    assert.equal(h.tree().node(tabOf(h, "A")).props.rect.y, 386);
    assert.equal(h.tree().node(textNode(h, "\u2715")).props.rect.y, 391);
    assert.equal(h.windows.get(a).visible, false);
    assert.equal(h.stacking().at(-1), b);
    assert.deepEqual(groupOf(h, b).windows, [a, b]);

    // Over another window's top band, the strip shrinks with the window, staying on top of it.
    dragOver(h, b, [{ x: 700, y: 170 }]);
    assert.deepEqual(transformOf(h, b), { scale: 0.375, x: -220, y: -218 });
    assert.deepEqual(h.tree().node(stripOf(h, "A")).props.transform, {
        scale: 0.375,
        x: -220,
        y: -81.75,
    });
    dragOver(h, b, [{ x: 700, y: 400 }]);
    assert.deepEqual(h.tree().node(stripOf(h, "A")).props.transform, { scale: 1, x: 0, y: 0 });

    // Dropped, the whole group is there.
    dragEnd(h, b);
    assert.deepEqual(h.rect(b), rect(600, 418, 640, 400));
    assert.deepEqual(h.rect(a), h.rect(b));
    assert.equal(h.windows.get(a).visible, false);
    assert.deepEqual(groupOf(h, b).windows, [a, b]);

    // Dropped on another window's top band, the whole group joins it.
    h.pointer(stripOf(h, "A"), "pointerdown");
    h.drag(b, ws, [{ x: 700, y: 170 }], true, ON_STRIP);
    assert.deepEqual(new Set(groupOf(h, b).windows), new Set([a, b, c]));
    assert.equal(groupOf(h, b).active, b);
    assert.deepEqual(h.rect(b), rect(352, 192, 640, 400));
    assert.deepEqual(h.exposedState().groups.length, 1);
});

test("a group's buttons minimize, maximize and ungroup it", async () => {
    const h = await load();
    const a = h.open({ size: SIZE, title: "A" });
    const b = h.open({ size: SIZE, title: "B" });
    const ws = h.activeWorkspace().id;
    h.drag(b, ws, [ABOVE]);

    // Maximized, the strip is at the top of the working area, the windows under it.
    h.pointer(h.tree().node(textNode(h, "\u25a1")).parent, "click");
    assert.deepEqual(h.rect(b), rect(0, 36, 1280, 684));
    assert.deepEqual(h.rect(a), h.rect(b));
    h.pointer(h.tree().node(textNode(h, "\u2750")).parent, "click");
    assert.deepEqual(h.rect(b), CENTRED);

    // Close ungroups: each window on its own, the others a step down and to the right.
    h.pointer(h.tree().node(textNode(h, "\u2715")).parent, "click");
    assert.equal(groupOf(h, a), null);
    assert.equal(h.windows.get(a).visible, true);
    assert.deepEqual(h.rect(b), CENTRED);
    assert.deepEqual(h.rect(a), rect(352, 192, 640, 400));

    h.drag(b, ws, [{ x: 650, y: 175 }]);
    assert.notEqual(groupOf(h, b), null);
    h.pointer(h.tree().node(textNode(h, "\u2013")).parent, "click");
    assert.equal(h.windows.get(b).visible, false);
    assert.equal(h.exposedState().groups[0].tabs, false);
});

test("bento's own group actions show as tabs too", async () => {
    const h = await load();
    const a = h.open({ size: SIZE, title: "A" });
    const b = h.open({ size: SIZE, title: "B" });
    h.createGroup(100, [a, b], b);
    h.flush();
    h.send({ type: "sync" });
    assert.equal(h.windows.get(a).visible, false);
    assert.deepEqual(h.rect(a), h.rect(b));
    assert.equal(h.exposedState().groups[0].tabs, true);
});
