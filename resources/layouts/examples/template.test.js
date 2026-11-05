// Tests for the template layout engine, `template.js`. Run them with node, without bento:
//
//     node --test resources/layouts/examples/
//
// `../test/host.js` plays bento's role: it opens and closes windows, runs actions and shows where
// the engine puts everything. The output is 1280 × 720 with 16 px gaps.

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

/** What the engine draws on the active workspace: the boxes. */
function boxes(host) {
    const tree = host.tree();
    return tree
        .order()
        .filter((id) => tree.node(id).kind === "box")
        .map((id) => tree.node(id).props);
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
    assert.equal(host.focused(), 3);
});

test("promote on the main window swaps it with the top of the stack", async () => {
    const host = await setUpThree();
    host.activate(1);
    host.action("promote");
    assert.deepEqual(host.rect(1), rect(710, 16, 554, 336));
    assert.deepEqual(host.rect(3), rect(16, 16, 678, 688));
});

test("promote swaps the focused window with the main one", async () => {
    const host = await setUpThree();
    host.activate(2);
    host.action("promote");
    assert.deepEqual(host.rect(2), rect(16, 16, 678, 688));
    assert.deepEqual(host.rect(3), rect(710, 16, 554, 336));
    assert.deepEqual(host.rect(1), rect(710, 368, 554, 336));
    assert.equal(host.focused(), 2);
});

test("the option sets the main window's width, and changes apply right away", async () => {
    const host = await setUpThree({ options: { "main-width": 0.5 } });
    assert.deepEqual(host.rect(1), rect(16, 16, 616, 688));

    host.setOptions({ "main-width": 0.7 });
    assert.deepEqual(host.rect(1), rect(16, 16, 862, 688));
    assert.deepEqual(host.rect(3), rect(894, 16, 370, 336));
});

test("a line is drawn between the main window and the stack", async () => {
    const host = await load();
    host.open();
    assert.equal(boxes(host).length, 0);

    host.open();
    assert.deepEqual(
        boxes(host).map((box) => box.rect),
        [rect(701, 16, 2, 688)],
    );

    host.close(2);
    assert.equal(boxes(host).length, 0);
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
    assert.equal(host.focused(), 2);
    assert.deepEqual(host.rect(2), rect(710, 16, 554, 688));
});

test("struts from the config leave space around the windows", async () => {
    const host = await load({ config: { struts: { left: 100, right: 0, top: 50, bottom: 0 } } });
    host.open();
    assert.deepEqual(host.rect(1), rect(116, 66, 1148, 638));
});

test("each workspace has its own order", async () => {
    const host = await setUpThree();
    const other = host.open({ workspace: host.workspace(1).id });
    assert.deepEqual(host.rect(other), rect(16, 16, 1248, 688));
    assert.deepEqual(host.rect(1), rect(16, 16, 678, 688));

    // A window that bento moves to another workspace goes on top of its stack.
    host.move(3, host.workspace(1).id);
    assert.deepEqual(host.rect(3), rect(710, 16, 554, 688));
    assert.deepEqual(host.rect(2), rect(710, 16, 554, 688));
});

test("reloading keeps the order", async () => {
    const host = await setUpThree();
    host.activate(2);
    host.action("promote");
    const before = [1, 2, 3].map((id) => host.rect(id));

    host.reload();
    assert.deepEqual(
        [1, 2, 3].map((id) => host.rect(id)),
        before,
    );
    assert.deepEqual(host.errors, []);
});
