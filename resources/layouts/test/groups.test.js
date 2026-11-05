// Tests for groups: bento's state, the objects and events the engine gets, and the calls it makes.
// They use small engines written here, so what's tested is the prelude and the test host.

import assert from "node:assert/strict";
import { test } from "node:test";

import { TestHost } from "./host.js";

/** A host running an API 2 engine made by `create`. */
function host(create, opts = {}) {
    return new TestHost({ meta: { api: 2, name: "test" }, default: create }, opts);
}

/** Places each window that's added side by side, 100 wide, and focuses it. */
function placing(bento) {
    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        win.place({ x: win.workspace.windows.length * 110 - 110, y: 0, w: 100, h: 100 });
        win.focus();
    });
}

/** Places windows, and records in `log` what groups tell the engine. */
function watching(log) {
    return (bento) => {
        placing(bento);
        bento.addEventListener("groupadd", (e) => {
            const group = e.group;
            const ids = () => [group.id, group.windows.map((w) => w.id), group.active.id];
            log.push(["groupadd", ...ids()]);
            group.addEventListener("change", () => log.push(["change", ...ids()]));
            group.addEventListener("remove", () => log.push(["remove", group.id]));
        });
    };
}

test("groups are objects on windows and on bento", () => {
    const h = host((bento) => {
        placing(bento);
        bento.exposeState(() => ({
            groups: bento.groups.map((g) => [g.id, g.windows.map((w) => w.id), g.active.id]),
            of: bento.windows.map((w) => w.group?.id ?? null),
        }));
    });
    const [a, , c] = [h.open(), h.open(), h.open()];
    assert.deepEqual(h.exposedState(), { groups: [], of: [null, null, null] });

    h.createGroup(5, [a, c], c);
    h.flush();
    assert.deepEqual(h.exposedState(), { groups: [[5, [a, c], c]], of: [5, null, 5] });
});

test("the engine makes a group, which bento keeps", () => {
    let group = null;
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("action", () => {
            group = bento.createGroup(bento.windows.slice(0, 2), bento.windows[1]);
        });
    });
    const [a, b] = [h.open(), h.open()];
    h.open();
    h.action("group");

    assert.deepEqual(h.groups, [{ id: group.id, windows: [a, b], active: b }]);
    assert.deepEqual([group.windows.map((w) => w.id), group.active.id], [[a, b], b]);
});

test("group methods change bento's groups and the engine's view right away", () => {
    const log = [];
    let seen = null;
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("action", (e) => {
            const [a, b, c] = bento.windows;
            if (e.name === "make") {
                const group = bento.createGroup([a, b]);
                seen = { windows: group.windows.map((w) => w.id), active: group.active.id, of: a.group === group };
            } else if (e.name === "add") {
                a.group.add(c);
            } else if (e.name === "activate") {
                a.group.activate(c);
            } else if (e.name === "remove") {
                a.group.remove(b);
            } else if (e.name === "dissolve") {
                a.group.dissolve();
            }
        });
        bento.addEventListener("groupadd", (e) => {
            e.group.addEventListener("change", () => log.push("change"));
            e.group.addEventListener("remove", () => log.push("remove"));
        });
    });
    const [a, b, c] = [h.open(), h.open(), h.open()];

    h.action("make");
    assert.deepEqual(seen, { windows: [a, b], active: a, of: true });
    assert.deepEqual(h.groups.map((g) => g.windows), [[a, b]]);

    h.action("add");
    assert.deepEqual(h.groups[0].windows, [a, b, c]);

    h.action("activate");
    assert.equal(h.groups[0].active, c);

    h.action("remove");
    assert.deepEqual(h.groups[0].windows, [a, c]);
    assert.equal(h.groups[0].active, c);

    h.action("dissolve");
    assert.deepEqual(h.groups, []);
    assert.deepEqual(log, ["change", "change", "change", "remove"]);
});

test("a group of one window is not made, and a window is in one group at most", () => {
    let made = [];
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("action", (e) => {
            const [a, b, c] = bento.windows;
            if (e.name === "one") {
                made.push(bento.createGroup([a, a]));
            } else {
                made.push(bento.createGroup([a, b]), bento.createGroup([b, c]));
            }
        });
    });
    const [, b, c] = [h.open(), h.open(), h.open()];

    h.action("one");
    assert.deepEqual(made, [null]);
    assert.deepEqual(h.groups, []);

    made = [];
    h.action("two");
    // b went to the second group, which left a alone: the first group is gone.
    assert.deepEqual(h.groups.map((g) => g.windows), [[b, c]]);
    assert.equal(made[0].windows.length, 1);
    assert.equal(made[1].windows.length, 2);
});

test("a group that's gone refuses changes", () => {
    let result = null;
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("action", () => {
            const [a, b, c] = bento.windows;
            const group = bento.createGroup([a, b]);
            group.dissolve();
            result = [group.add(c), group.remove(a), group.activate(b), bento.groups.length];
        });
    });
    h.open();
    h.open();
    h.open();
    h.action("go");
    assert.deepEqual(result, [false, false, false, 0]);
    assert.deepEqual(h.groups, []);
});

test("only the engine's windows can be grouped, and the active one must be in the group", () => {
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("action", () => {
            const [a, b, c] = bento.windows;
            assert.throws(() => bento.createGroup([a, {}]), /not a window of this session/);
            assert.throws(() => bento.createGroup([a, b], c), /one of its windows/);
        });
    });
    h.open();
    h.open();
    h.open();
    h.action("go");
});

test("bento's group actions reach the engine as events", () => {
    const log = [];
    const h = host(watching(log));
    const [, b, c] = [h.open(), h.open(), h.open()];

    // The focused window, c, makes a group with the window to its left, and is active in it.
    h.groupWindow("left");
    const id = h.groups[0].id;
    assert.deepEqual(h.groups, [{ id, windows: [b, c], active: c }]);
    assert.deepEqual(log, [["groupadd", id, [b, c], c]]);

    // b is the nearest window still, and c is in its group already.
    h.groupWindow("left");
    assert.equal(log.length, 1);

    h.ungroupWindow();
    assert.deepEqual(h.groups, []);
    assert.deepEqual(log.at(-1), ["remove", id]);
});

test("a window joins the group of the window next to it", () => {
    const log = [];
    const h = host(watching(log));
    const [a, b, c] = [h.open(), h.open(), h.open()];
    h.createGroup(h.nextGroupId++, [a, b], a);
    h.flush();
    log.length = 0;

    h.groupWindow("left");
    const id = h.groups[0].id;
    assert.deepEqual(h.groups, [{ id, windows: [a, b, c], active: c }]);
    assert.deepEqual(log, [["change", id, [a, b, c], c]]);
});

test("a closing window leaves its group, which is gone at one window", () => {
    const log = [];
    const h = host(watching(log));
    const [a, b, c] = [h.open(), h.open(), h.open()];
    h.createGroup(h.nextGroupId++, [a, b, c], b);
    h.flush();
    const id = h.groups[0].id;
    log.length = 0;

    h.close(b);
    // The window that took b's place in the order is active now.
    assert.deepEqual(log, [["change", id, [a, c], c]]);

    h.close(a);
    assert.deepEqual(h.groups, []);
    assert.deepEqual(log.at(-1), ["remove", id]);
});

test("a window dragged out of the engine leaves its group", () => {
    const h = host(placing);
    const [a, b] = [h.open(), h.open()];
    h.createGroup(h.nextGroupId++, [a, b], a);
    h.flush();

    h.drag(b, h.activeWorkspace().id, [{ x: 5, y: 5 }]);
    assert.deepEqual(h.groups, []);
});

test("changes the engine makes reach its listeners once", () => {
    const log = [];
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("action", () => {
            const group = bento.createGroup(bento.windows.slice(0, 2));
            group.addEventListener("change", () => log.push("change"));
        });
    });
    const [a, b, c] = [h.open(), h.open(), h.open()];
    h.action("go");
    assert.equal(h.groups.length, 1);
    assert.deepEqual(log, []);

    h.groupWindow("left");
    assert.deepEqual(h.groups[0].windows, [a, b, c]);
    assert.deepEqual(log, ["change"]);
});

test("a reloaded engine learns the groups that bento has, after its windows", () => {
    const seen = [];
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("groupadd", (e) => seen.push(e.group.id));
        bento.exposeState(() => bento.windows.map((w) => w.group?.id ?? null));
    });
    const [a, , c] = [h.open(), h.open(), h.open()];
    h.createGroup(9, [a, c], a);
    h.flush();
    assert.deepEqual(seen, [9]);

    h.reload();
    assert.deepEqual(seen, [9, 9]);
    assert.deepEqual(h.exposedState(), [9, null, 9]);
});
