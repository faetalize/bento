// Tests for the template layout engine, `template.js`. Run them with node, without bento:
//
//     node --test resources/layouts/examples/
//
// `../test/host.js` plays bento's role: it opens and closes windows, runs actions and swipes, and
// shows where the engine puts everything. The output is 1280 × 720 with 16 px gaps.

import { test } from "node:test";
import assert from "node:assert/strict";

import { TestHost } from "../test/host.js";

// The path is relative to `host.js`.
const load = (opts) => TestHost.load("../examples/template.js", opts);
const rect = (x, y, w, h) => ({ x, y, w, h });

/** Three windows: 1 is the main window, 3 on top of the stack and 2 below it, 3 focused. */
async function setUpThree(opts) {
    const host = await load(opts);
    host.open();
    host.open();
    host.open();
    return host;
}

test("the main window takes the left, the others share the right", async () => {
    const host = await load();
    host.open();
    assert.deepEqual(host.rect(1), rect(16, 16, 1248, 688));

    host.open();
    // (1280 - 3 * 16) * 0.55 rounds to 678.
    assert.deepEqual(host.rect(1), rect(16, 16, 678, 688));
    assert.deepEqual(host.rect(2), rect(710, 16, 554, 688));

    // New windows go on top of the stack.
    host.open();
    assert.deepEqual(host.rect(3), rect(710, 16, 554, 336));
    assert.deepEqual(host.rect(2), rect(710, 368, 554, 336));
});

test("left and right go between the main window and the stack, up and down within it", async () => {
    const host = await setUpThree();
    assert.equal(host.focused, 3);

    assert.equal(host.action("focus-window-down"), true);
    assert.equal(host.focused, 2);
    assert.equal(host.action("focus-column-left"), true);
    assert.equal(host.focused, 1);

    // Nothing further left, so bento can fall back, like to the monitor on the left.
    assert.equal(host.action("focus-column-left"), false);
    assert.equal(host.action("focus-column-right"), true);
    assert.equal(host.focused, 3);
});

test("actions that combine others do their part within the workspace", async () => {
    const host = await setUpThree();
    assert.equal(host.action("focus-column-or-monitor-left"), true);
    assert.equal(host.focused, 1);
    // Nothing further left: bento focuses the monitor to the left, if there's one.
    assert.equal(host.action("focus-column-or-monitor-left"), false);
    assert.equal(host.action("move-column-right-or-to-monitor-right"), true);
    assert.deepEqual(host.rect(1), rect(710, 16, 554, 336));
});

test("promote swaps the focused window with the main one", async () => {
    const host = await setUpThree();
    host.action("focus-window-down");
    assert.equal(host.action("promote"), true);
    assert.deepEqual(host.rect(2), rect(16, 16, 678, 688));
    assert.deepEqual(host.rect(3), rect(710, 16, 554, 336));
    assert.deepEqual(host.rect(1), rect(710, 368, 554, 336));
});

test("set-column-width changes the width of the main window", async () => {
    const host = await setUpThree();
    host.action("set-column-width", { op: "adjust", value: 10, unit: "%" });
    assert.deepEqual(host.rect(1), rect(16, 16, 801, 688));
    assert.deepEqual(host.rect(3), rect(833, 16, 431, 336));

    // The option sets where it starts.
    const other = await setUpThree({ options: { "main-width": 0.5 } });
    assert.deepEqual(other.rect(1), rect(16, 16, 616, 688));
});

test("fullscreen windows cover the view", async () => {
    const host = await setUpThree();
    host.requestMode(3, "fullscreen", { fullscreen: true });
    assert.deepEqual(host.rect(3), rect(0, 0, 1280, 720));
    assert.equal(host.mode(3), "fullscreen");

    host.requestMode(3, "normal", { fullscreen: false });
    assert.deepEqual(host.rect(3), rect(710, 16, 554, 336));
});

test("closing the focused window focuses the one that takes its place", async () => {
    const host = await setUpThree();
    host.close(3);
    assert.equal(host.focused, 2);
    assert.deepEqual(host.rect(2), rect(710, 16, 554, 688));
});

test("struts from the config leave space around the windows", async () => {
    const host = await load({ config: { struts: { left: 100, right: 0, top: 50, bottom: 0 } } });
    host.open();
    assert.deepEqual(host.rect(1), rect(116, 66, 1148, 638));
});

test("swiping doesn't move the view", async () => {
    const host = await setUpThree();
    host.swipe(300);
    assert.deepEqual(host.camera, { x: 0, y: 0 });
});

test("reloading keeps the order and the width", async () => {
    const host = await setUpThree();
    host.action("focus-window-down");
    host.action("promote");
    host.action("set-column-width", { op: "set", value: 40, unit: "%" });
    const before = [1, 2, 3].map((id) => host.rect(id));

    host.reload();
    assert.deepEqual(
        [1, 2, 3].map((id) => host.rect(id)),
        before,
    );
});
