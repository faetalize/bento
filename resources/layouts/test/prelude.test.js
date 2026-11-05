// Tests for the prelude: events, objects, queries, the node ops and the API 1 adapter. They use
// small engines written here, so what's tested is the prelude, not an engine.

import assert from "node:assert/strict";
import { test } from "node:test";

import * as prelude from "../prelude/index.js";
import { TestHost } from "./host.js";

const TWO_MONITORS = [{ name: "DP-1" }, { name: "DP-2" }];

/** A host running an API 2 engine made by `create`. */
function host(create, opts = {}, meta = {}) {
    return new TestHost({ meta: { api: 2, name: "test", ...meta }, default: create }, opts);
}

/** A host running an API 1 engine made by `create`. */
function hostV1(create, opts = {}, meta = {}) {
    return new TestHost({ meta: { api: 1, name: "test-v1", ...meta }, default: create }, opts);
}

/** Places each window that's added, and focuses it. */
function placing(bento, rect = { x: 0, y: 0, w: 10, h: 10 }) {
    bento.addEventListener("windowadd", (e) => {
        e.window.place(rect);
        e.window.focus();
    });
}

// Loading.

test("load checks the module and describes the engine", () => {
    const meta = JSON.parse(
        prelude.load({
            meta: {
                api: 2,
                name: "described",
                description: "d",
                floating: false,
                actions: {
                    go: {
                        description: "Goes.",
                        args: [
                            { name: "to", type: "workspace" },
                            { name: "on", type: "monitor", default: "DP-1" },
                        ],
                    },
                },
                options: {
                    widths: { type: "size", list: true, default: [{ proportion: 0.5 }] },
                    mode: { type: "choice", choices: ["a", "b"], default: "a" },
                },
                windowRules: { width: { type: "size", default: null } },
            },
            default() {},
        }),
    );
    assert.equal(meta.name, "described");
    assert.equal(meta.floating, false);
    assert.deepEqual(
        meta.actions[0].args.map((a) => [a.name, a.type, a.default]),
        [
            ["to", "workspace", null],
            ["on", "monitor", "DP-1"],
        ],
    );
    assert.deepEqual(
        meta.options.map((o) => [o.name, o.type, o.list]),
        [
            ["widths", "size", true],
            ["mode", "choice", false],
        ],
    );
    assert.deepEqual(meta.options[1].choices, ["a", "b"]);
    assert.equal(meta.windowRules[0].name, "width");
});

test("load refuses engines that don't fit", () => {
    const load = (meta, fn = () => {}) => () => prelude.load({ meta, default: fn });
    assert.throws(load({ api: 3, name: "x" }), /API version 3/);
    assert.throws(load({ api: 2 }), /must be a string/);
    assert.throws(load({ api: 2, name: "x" }, null), /default function/);
    assert.throws(load({ api: 2, name: "x", actions: { a: { args: [{ name: "n", type: "nope" }] } } }), /unknown type/);
    assert.throws(
        load({ api: 2, name: "x", options: { o: { type: "choice", choices: ["a"], default: "b" } } }),
        /must be a choice/,
    );
    assert.throws(load({ api: 2, name: "x", options: { o: { type: "number", default: "1" } } }), /must be a number/);
    // Workspaces and monitors are API 2 types.
    assert.throws(
        load({ api: 1, name: "x", actions: { a: { args: [{ name: "n", type: "workspace" }] } } }),
        /unknown type/,
    );
});

// Events and objects.

test("a new window comes with its reason and gets placed", () => {
    const seen = [];
    const h = host((bento) => {
        bento.addEventListener("windowadd", (e) => {
            seen.push([e.window.id, e.reason, e.activate, e.rect, e.nextTo]);
            e.window.place({ x: 0, y: 0, w: 100, h: 50 });
            e.window.focus();
        });
    });
    const a = h.open();
    assert.deepEqual(seen, [[a, "open", true, null, null]]);
    assert.deepEqual(h.rect(a), { x: 0, y: 0, w: 100, h: 50 });
    assert.equal(h.focused(), a);
});

test("the engine sees every monitor and workspace", () => {
    const h = host(
        (bento) => {
            bento.exposeState(() =>
                bento.monitors.map((m) => ({
                    name: m.name,
                    left: m.left?.name ?? null,
                    right: m.right?.name ?? null,
                    up: m.up?.name ?? null,
                    workspaces: m.workspaces.map((ws) => ws.index),
                    active: m.activeWorkspace.index,
                    focused: m.focused,
                })),
            );
        },
        { monitors: TWO_MONITORS },
    );
    assert.deepEqual(h.exposedState(), [
        { name: "DP-1", left: null, right: "DP-2", up: null, workspaces: [0, 1], active: 0, focused: true },
        { name: "DP-2", left: "DP-1", right: null, up: null, workspaces: [0, 1], active: 0, focused: false },
    ]);
});

test("monitors and workspaces that come and go are events", () => {
    const seen = [];
    const h = host((bento) => {
        bento.addEventListener("monitoradd", (e) => {
            seen.push(["monitoradd", e.monitor.name]);
            e.monitor.addEventListener("remove", () => seen.push(["monitor remove", e.monitor.name]));
        });
        bento.addEventListener("workspaceadd", (e) => {
            seen.push(["workspaceadd", e.workspace.monitor.name, e.workspace.index]);
            e.workspace.addEventListener("remove", () => seen.push(["workspace remove", e.workspace.index]));
        });
    });
    h.addMonitor({ name: "DP-2", size: { w: 800, h: 600 }, scale: 1, workspaces: 1, position: { x: 1280, y: 0 } });
    h.flush();
    assert.deepEqual(seen.splice(0), [
        ["monitoradd", "DP-2"],
        ["workspaceadd", "DP-2", 0],
    ]);

    h.removeMonitor("DP-2");
    assert.deepEqual(seen, [
        ["monitor remove", "DP-2"],
        ["workspace remove", 0],
    ]);
});

test("workspaces tell when they're shown or hidden", () => {
    const changes = [];
    const h = host((bento) => {
        for (const ws of bento.monitors[0].workspaces) {
            ws.addEventListener("change", () => changes.push(`${ws.index}:${ws.active}`));
        }
    });
    h.showWorkspace(h.workspace(1).id);
    assert.deepEqual(changes, ["0:false", "1:true"]);
});

test("focuschange tells when keyboard focus moves", () => {
    let focuses = 0;
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("focuschange", () => focuses++);
    });
    const a = h.open();
    h.open();
    assert.equal(focuses, 2);
    h.activate(a);
    assert.equal(focuses, 3);
});

test("a window's changes are events", () => {
    const titles = [];
    const h = host((bento) => {
        bento.addEventListener("windowadd", (e) => {
            e.window.place({ x: 0, y: 0, w: 10, h: 10 });
            e.window.addEventListener("change", () => titles.push([e.window.title, e.window.minSize.w]));
        });
    });
    const a = h.open({ title: "one" });
    h.setTitle(a, "two");
    h.setLimits(a, { minSize: { w: 30, h: 0 } });
    // The first change is the window taking the size it was placed at.
    assert.deepEqual(titles, [
        ["one", 0],
        ["two", 0],
        ["two", 30],
    ]);
});

test("activaterequest can be prevented, and bento's default focuses the window", () => {
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("windowadd", (e) => {
            if (e.window.title === "locked") {
                e.window.addEventListener("activaterequest", (r) => r.preventDefault());
            }
        });
    });
    const locked = h.open({ title: "locked" });
    const b = h.open();
    h.activate(locked);
    assert.equal(h.verdicts.at(-1).prevented, true);
    assert.equal(h.focused(), b);

    const c = h.open();
    h.activate(b);
    assert.equal(h.verdicts.at(-1).prevented, false);
    assert.equal(h.focused(), b);
    assert.notEqual(c, b);
});

test("moderequest takes the mode unless it's prevented", () => {
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("windowadd", (e) => {
            e.window.addEventListener("moderequest", (r) => {
                if (e.window.title === "no") r.preventDefault();
            });
        });
    });
    const a = h.open();
    const b = h.open({ title: "no" });
    h.requestMode(a, "fullscreen");
    h.requestMode(b, "fullscreen");
    assert.equal(h.mode(a), "fullscreen");
    assert.equal(h.mode(b), "normal");
});

test("a moderequest event says exactly what changes", () => {
    const requests = [];
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("windowadd", (e) => {
            e.window.addEventListener("moderequest", (r) => requests.push([r.mode, r.request]));
        });
    });
    const a = h.open();
    h.requestMode(a, "normal", { fullscreen: false });
    assert.deepEqual(requests, [["normal", { fullscreen: false }]]);
});

test("resizing is refused without a listener, or when one prevents it", () => {
    const deltas = [];
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("windowadd", (e) => {
            if (e.window.title === "plain") return;
            e.window.addEventListener("resizebegin", (r) => {
                if (e.window.title === "no") r.preventDefault();
            });
            e.window.addEventListener("resizeupdate", (r) => deltas.push([r.edges.right, r.delta.x]));
        });
    });
    const plain = h.open({ title: "plain" });
    const no = h.open({ title: "no" });
    const yes = h.open({ title: "yes" });
    assert.equal(h.resize(plain, { right: true }, { x: 5, y: 0 }), false);
    assert.equal(h.resize(no, { right: true }, { x: 5, y: 0 }), false);
    assert.equal(h.resize(yes, { right: true }, { x: 7, y: 0 }), true);
    assert.deepEqual(deltas, [[true, 7]]);
});

test("a double click on a resize edge is an event", () => {
    const edges = [];
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("windowadd", (e) => {
            e.window.addEventListener("edgedoubleclick", (c) => edges.push(c.edges));
        });
    });
    const a = h.open();
    h.doubleClickEdge(a, { left: true });
    assert.deepEqual(edges, [{ left: true, right: false, top: false, bottom: false }]);
});

test("floatrequest reaches the window", () => {
    const seen = [];
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("windowadd", (e) => {
                e.window.addEventListener("floatrequest", (f) => seen.push([f.reason, f.parent?.id ?? null, f.size]));
            });
        },
        {},
        { floating: false },
    );
    const a = h.open();
    const b = h.open();
    h.send({ type: "floatrequest", id: b, reason: "dialog", parent: a, size: { w: 200, h: 100 } });
    assert.deepEqual(seen, [["dialog", a, { w: 200, h: 100 }]]);
});

test("queries get the first answer and change nothing", () => {
    const h = host((bento) => {
        placing(bento);
        bento.addEventListener("newwindowsize", (e) => {
            for (const win of e.workspace.windows) {
                win.place({ x: 1, y: 1, w: 1, h: 1 });
                win.focus();
            }
            e.workspace.camera.move({ x: 500 });
            e.respondWith({ w: e.workspace.tilingArea.w / 2, h: 100 });
            e.respondWith({ w: 1, h: 1 });
        });
    });
    const a = h.open();
    h.open();
    h.activate(a);
    assert.deepEqual(h.newWindowSize(), { w: 640, h: 100 });
    assert.deepEqual(h.rect(a), { x: 0, y: 0, w: 10, h: 10 });
    assert.equal(h.focused(), a);
    assert.deepEqual(h.camera(), { x: 0, y: 0 });
});

test("a query without an answer says 0 × 0", () => {
    const h = host(() => {});
    assert.deepEqual(h.newWindowSize(), { w: 0, h: 0 });
});

test("state is saved on unload, and windows are adopted when the engine starts again", () => {
    let saved = "none";
    let adopted = [];
    const h = host((bento) => {
        saved = bento.savedState;
        bento.addEventListener("windowadd", (e) => {
            adopted.push([e.window.id, e.reason, e.activate, e.rect]);
            e.window.place({ x: 5, y: 5, w: 50, h: 50 });
            e.window.focus();
        });
        bento.addEventListener("unload", (e) => e.saveState({ n: 42 }));
    });
    assert.equal(saved, null);
    const a = h.open();
    const b = h.open();
    adopted = [];

    h.reload();
    assert.deepEqual(saved, { n: 42 });
    const rect = { x: 5, y: 5, w: 50, h: 50 };
    assert.deepEqual(adopted, [
        [a, "adopt", false, rect],
        [b, "adopt", true, rect],
    ]);
});

test("options are filled in from the declarations, and checked", () => {
    const seen = [];
    const h = host(
        (bento) => {
            bento.addEventListener("configchange", () => seen.push({ ...bento.options }));
            bento.exposeState(() => ({ ...bento.options }));
        },
        { options: { a: 2, nope: 1, b: "x" } },
        {
            options: {
                a: { type: "number", default: 1 },
                b: { type: "boolean", default: true },
                c: { type: "string", default: "c" },
            },
        },
    );
    assert.deepEqual(h.exposedState(), { a: 2, b: true, c: "c" });
    assert.match(h.logs.join("\n"), /unknown layout engine option: nope/);
    assert.match(h.logs.join("\n"), /option b must be a boolean/);

    h.setOptions({ a: 3 });
    assert.deepEqual(seen, [{ a: 3, b: true, c: "c" }]);
});

test("workspaces report their config and tiling area", () => {
    const h = host(
        (bento) => {
            bento.exposeState(() => {
                const ws = bento.monitors[0].workspaces[0];
                return [ws.config, ws.tilingArea];
            });
        },
        { config: { struts: { left: 10, right: 20, top: 30, bottom: 40 } } },
    );
    const [config, area] = h.exposedState();
    assert.equal(config.gaps, 16);
    assert.deepEqual(area, { x: 10, y: 30, w: 1250, h: 650 });
});

test("action arguments are turned into objects by their types", () => {
    const got = [];
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("action", (e) => {
                got.push([e.name, e.args[0]?.id ?? null, e.args[1]?.name ?? null, e.args[2], e.window?.id ?? null]);
            });
        },
        { monitors: TWO_MONITORS },
        {
            actions: {
                go: {
                    args: [
                        { name: "to", type: "workspace" },
                        { name: "on", type: "monitor" },
                        { name: "n", type: "number", default: 3 },
                    ],
                },
            },
        },
    );
    const a = h.open();
    const target = h.workspace(1, "DP-2").id;
    h.action("go", target, "DP-2", 3);
    h.action("go", 999, "DP-9", 3);
    assert.deepEqual(got, [
        ["go", target, "DP-2", 3, a],
        ["go", null, null, 3, a],
    ]);
});

test("a window can be moved to a workspace on another monitor", () => {
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("action", (e) => {
                e.window.moveTo(bento.monitors[e.args[0]].workspaces[e.args[1]]);
            });
        },
        { monitors: TWO_MONITORS },
        { actions: { send: { args: [{ name: "monitor", type: "number" }, { name: "workspace", type: "number" }] } } },
    );
    const a = h.open();
    const target = h.workspace(1, "DP-2").id;
    h.action("send", 1, 1);
    assert.equal(h.windows.get(a).workspace, target);
    assert.deepEqual(h.stacking(target), [a]);
    assert.deepEqual(h.stacking(h.workspace(0, "DP-1").id), []);
    assert.equal(h.workspaces.get(h.workspace(0, "DP-1").id).focused, null);
});

test("windows that bento moves are told with a move event, and their node goes along", () => {
    const moves = [];
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("windowadd", (e) => {
                e.window.addEventListener("move", (m) => moves.push([m.from.index, m.to.index, m.activate, e.window.workspace.index]));
            });
        },
    );
    const a = h.open();
    const second = h.workspace(1).id;
    h.move(a, second, true);
    assert.deepEqual(moves, [[0, 1, true, 1]]);
    assert.deepEqual(h.stacking(second), [a]);
});

test("workspaces can be shown, renamed and moved", () => {
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("action", (e) => {
                const ws = bento.monitors[0].workspaces[1];
                if (e.name === "show") ws.show({ animate: false });
                if (e.name === "rename") ws.rename("two");
                if (e.name === "send") ws.moveTo(bento.monitors[1]);
            });
        },
        { monitors: TWO_MONITORS },
        { actions: { show: {}, rename: {}, send: {} } },
    );
    h.open();
    const second = h.workspace(1).id;
    h.action("show");
    assert.equal(h.activeWorkspace().id, second);
    h.action("rename");
    assert.equal(h.workspaces.get(second).name, "two");
    h.action("send");
    assert.equal(h.workspaces.get(second).monitor, "DP-2");
    assert.equal(h.stack("DP-2").length, 3);
    assert.equal(h.stack("DP-1").length, 1);
});

test("the engine can focus a monitor", () => {
    const h = host(
        (bento) => {
            bento.addEventListener("action", () => bento.monitors[1].focus());
        },
        { monitors: TWO_MONITORS },
        { actions: { go: {} } },
    );
    h.action("go");
    assert.equal(h.focusedMonitor, "DP-2");
});

test("the camera moves with the options the engine gives", () => {
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("action", (e) => {
                const ws = e.window.workspace;
                ws.camera.move({ x: 100 }, { animate: false });
                ws.camera.move({ y: 5 }, { velocity: 300 });
                assert.deepEqual([ws.camera.x, ws.camera.y], [100, 5]);
            });
        },
        {},
        { actions: { go: {} } },
    );
    h.open();
    h.action("go");
    assert.deepEqual(h.camera(), { x: 100, y: 5 });
    assert.equal(h.lastCamera.velocity, 300);
    assert.equal(h.lastCamera.animate, true);
});

test("horizontal swipes are reported, and the engine moves the camera", () => {
    const h = host((bento) => {
        for (const ws of bento.monitors[0].workspaces) {
            ws.addEventListener("swipeupdate", (e) => ws.camera.move({ x: ws.camera.x - e.delta }, { animate: false }));
            ws.addEventListener("swipeend", (e) =>
                ws.camera.move({ x: Math.round(ws.camera.x / 100) * 100 }, { velocity: e.velocity }),
            );
        }
    });
    h.swipe(h.workspace(0).id, [30, 40], 500);
    assert.deepEqual(h.camera(), { x: -100, y: 0 });
    assert.equal(h.lastCamera.velocity, 500);
});

test("drags over a workspace are events, and the drop adds the window", () => {
    const seen = [];
    const h = host((bento) => {
        placing(bento);
        const ws = bento.monitors[0].workspaces[0];
        for (const type of ["dragenter", "dragmove", "dragleave"]) {
            ws.addEventListener(type, (e) => seen.push([type, e.window?.id ?? null, e.pointer?.x ?? null, e.rect?.w ?? null]));
        }
        bento.addEventListener("windowadd", (e) => {
            if (e.reason === "drop") seen.push(["drop", e.pointer.x, e.rect.x]);
        });
    });
    const a = h.open();
    h.drag(a, h.workspace(0).id, [{ x: 100, y: 10 }, { x: 120, y: 10 }]);
    assert.deepEqual(seen, [
        ["dragenter", a, 100, 20],
        ["dragmove", a, 120, 20],
        ["dragleave", null, null, null],
        ["drop", 120, 110],
    ]);
    assert.deepEqual(h.stacking(), [a]);
});

test("exposed state and its changes are readable", () => {
    const h = host(
        (bento) => {
            let n = 0;
            bento.exposeState(() => ({ n }));
            bento.addEventListener("action", () => {
                n++;
                bento.stateChanged();
            });
        },
        {},
        { actions: { bump: {} } },
    );
    assert.deepEqual(h.exposedState(), { n: 0 });
    h.action("bump");
    assert.deepEqual(h.exposedState(), { n: 1 });
    assert.equal(h.stateChanges, 1);
});

test("an engine that throws doesn't stop the others listening", () => {
    const seen = [];
    const h = host((bento) => {
        bento.addEventListener("windowadd", () => {
            throw new Error("broken");
        });
        bento.addEventListener("windowadd", (e) => seen.push(e.window.id));
    });
    assert.throws(() => h.open(), /broken/);
    assert.deepEqual(seen, [1]);
});

test("window objects can't be used after they're gone", () => {
    let gone;
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("windowadd", (e) => (gone = e.window));
            bento.addEventListener("action", () => gone.place({ x: 0, y: 0, w: 1, h: 1 }));
        },
        {},
        { actions: { go: {} } },
    );
    const a = h.open();
    h.close(a);
    assert.throws(() => h.action("go"), /not in the layout engine/);
});

// Nodes.

/** An engine with actions that draw a box and change it. */
function drawing(extra = () => {}) {
    const nodes = {};
    const h = host(
        (bento) => {
            placing(bento);
            nodes.bento = bento;
            nodes.box = bento.createNode("box", { rect: { x: 1, y: 2, w: 3, h: 4 }, background: "#fff" });
            extra(bento, nodes);
            bento.addEventListener("action", (e) => {
                const tree = e.window.workspace.tree;
                if (e.name === "draw") tree.append(nodes.box);
                if (e.name === "color") nodes.box.set({ background: "#000" });
                if (e.name === "hide") nodes.box.remove();
                if (e.name === "under") tree.append(nodes.box, e.window.node);
            });
        },
        {},
        { actions: { draw: {}, color: {}, hide: {}, under: {} } },
    );
    h.open();
    return { h, nodes };
}

test("a node reaches bento when it enters a tree", () => {
    const { h, nodes } = drawing();
    h.action("draw");
    const tree = h.tree();
    assert.match(nodes.box.id, /^n\d+$/);
    assert.deepEqual(tree.node(nodes.box.id).props, { rect: { x: 1, y: 2, w: 3, h: 4 }, background: "#fff" });
    assert.deepEqual(tree.node(tree.rootId).children, ["w1", nodes.box.id]);

    h.action("color");
    assert.equal(tree.node(nodes.box.id).props.background, "#000");
});

test("a node that changes before it enters a tree is created with its changes", () => {
    const { h, nodes } = drawing();
    h.action("color");
    assert.equal(h.tree().has(nodes.box.id), false);
    h.action("draw");
    assert.equal(h.tree().node(nodes.box.id).props.background, "#000");
});

test("a removed node is dropped, and created again when it comes back", () => {
    const { h, nodes } = drawing();
    h.action("draw");
    h.action("hide");
    assert.equal(h.tree().has(nodes.box.id), false);
    assert.equal(nodes.box.parent, null);
    h.action("color");
    h.action("draw");
    assert.equal(h.tree().node(nodes.box.id).props.background, "#000");
});

test("a node can go before another", () => {
    const { h, nodes } = drawing();
    h.action("under");
    assert.deepEqual(h.tree().node(h.tree().rootId).children, [nodes.box.id, "w1"]);
});

test("a subtree is created with its nodes, in order", () => {
    const { h } = drawing((bento, nodes) => {
        bento.addEventListener("action", (e) => {
            if (e.name !== "draw") return;
            const inner = bento.createNode("text", { text: "hi" });
            nodes.inner = inner;
            nodes.box.append(inner);
        });
    });
    h.action("draw");
    const tree = h.tree();
    assert.deepEqual(tree.node(h.tree().rootId).children, ["w1", "n1"]);
    assert.equal(tree.node("n2").kind, "text");
    assert.deepEqual(tree.node("n1").children, ["n2"]);
});

test("window nodes keep the order of the windows, and raise moves them last", () => {
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("action", () => bento.windows[0].raise());
        },
        {},
        { actions: { go: {} } },
    );
    const [a, b, c] = [h.open(), h.open(), h.open()];
    assert.deepEqual(h.stacking(), [a, b, c]);
    h.action("go");
    assert.deepEqual(h.stacking(), [b, c, a]);
});

test("a window's node can't be removed", () => {
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("action", () => bento.windows[0].node.remove());
        },
        {},
        { actions: { go: {} } },
    );
    h.open();
    assert.throws(() => h.action("go"), /can't be removed/);
});

test("placing a window sets its node's rect", () => {
    const h = host((bento) => placing(bento, { x: 7, y: 8, w: 9, h: 10 }));
    const a = h.open();
    assert.deepEqual(h.tree().node(`w${a}`).props.rect, { x: 7, y: 8, w: 9, h: 10 });
});

test("the nodes of a workspace's tree are the engine's, and windows can sit in them", () => {
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("action", (e) => {
                const box = bento.createNode("box");
                e.window.workspace.tree.append(box);
                box.append(e.window.node);
            });
        },
        {},
        { actions: { go: {} } },
    );
    const a = h.open();
    h.action("go");
    assert.deepEqual(h.stacking(), [a]);
    assert.deepEqual(h.tree().node("n1").children, [`w${a}`]);
});

test("a window moved to another workspace takes what's drawn on it", () => {
    const h = host(
        (bento) => {
            placing(bento);
            bento.addEventListener("action", (e) => {
                const badge = bento.createNode("box");
                e.window.node.append(badge);
                e.window.moveTo(e.window.workspace.monitor.workspaces[1]);
            });
        },
        {},
        { actions: { go: {} } },
    );
    const a = h.open();
    h.action("go");
    const second = h.workspace(1).id;
    assert.deepEqual(h.stacking(second), [a]);
    assert.deepEqual(h.tree(second).node(`w${a}`).children, ["n1"]);
});

test("pointer events bubble to the parents until a listener stops them", () => {
    const log = [];
    const { h, nodes } = drawing((bento, shared) => {
        bento.addEventListener("action", (e) => {
            if (e.name !== "draw") return;
            const inner = bento.createNode("box", { interactive: true });
            shared.inner = inner;
            shared.box.append(inner);
            shared.box.addEventListener("click", (c) => log.push(["outer", c.target === inner, c.button, c.modifiers.ctrl]));
            inner.addEventListener("click", (c) => {
                log.push(["inner"]);
                if (shared.stop) c.stopPropagation();
            });
        });
    });
    h.action("draw");
    h.pointer(nodes.inner.id, "click", { button: 1, modifiers: { shift: false, ctrl: true, alt: false, super: false } });
    assert.deepEqual(log, [["inner"], ["outer", true, 1, true]]);

    log.length = 0;
    nodes.stop = true;
    h.pointer(nodes.inner.id, "click");
    assert.deepEqual(log, [["inner"]]);
});

// The API 1 adapter.

/** A V1 engine that places every window in the same spot, and records what it's told. */
function simple(log = []) {
    return (ws) => ({
        windowAdded(win, how) {
            log.push(["added", win.id, how.reason, how.activate]);
            ws.place(win, { x: 0, y: 0, w: 100, h: 100 });
            if (how.activate) ws.focus(win);
        },
        windowRemoved(win, how) {
            log.push(["removed", win.id, how.reason]);
        },
    });
}

test("an API 1 engine runs once for each workspace", () => {
    let created = 0;
    const h = hostV1(
        (ws) => {
            created++;
            return simple()(ws);
        },
        { monitors: TWO_MONITORS },
    );
    assert.equal(created, 4);

    h.addMonitor({ name: "DP-3", size: { w: 800, h: 600 }, scale: 1, workspaces: 1, position: { x: 2560, y: 0 } });
    h.flush();
    assert.equal(created, 5);
});

test("windows go to the instance of their workspace, and moves are a remove and an add", () => {
    const logs = [];
    let n = 0;
    const h = hostV1((ws) => {
        const log = [];
        logs.push(log);
        n++;
        return simple(log)(ws);
    });
    assert.equal(n, 2);
    const second = h.workspace(1).id;
    const a = h.open();
    const b = h.open({ workspace: second });
    assert.deepEqual(logs[0], [["added", a, "open", true]]);
    assert.deepEqual(logs[1], [["added", b, "open", true]]);
    assert.deepEqual(h.rect(a), { x: 0, y: 0, w: 100, h: 100 });
    assert.equal(h.workspaces.get(h.workspace(0).id).focused, a);

    h.move(a, second, false);
    assert.deepEqual(logs[0].at(-1), ["removed", a, "move"]);
    assert.deepEqual(logs[1].at(-1), ["added", a, "move", false]);

    h.close(a);
    assert.deepEqual(logs[1].at(-1), ["removed", a, "close"]);
});

test("an API 1 engine gets the existing windows through adopt, with its saved state", () => {
    const adopted = [];
    const h = hostV1((ws) => ({
        ...simple()(ws),
        adopt(windows, focused, state) {
            adopted.push([windows.map((w) => w.id), focused?.id ?? null, state]);
            for (const win of windows) ws.place(win, win.rect);
        },
        save() {
            return { saved: true };
        },
    }));
    const a = h.open();
    adopted.length = 0;

    h.reload();
    assert.deepEqual(adopted, [
        [[a], a, { saved: true }],
        [[], null, { saved: true }],
    ]);
});

test("an API 1 engine's queries and actions work", () => {
    const calls = [];
    const h = hostV1(
        (ws) => ({
            ...simple()(ws),
            action(name, args, target) {
                calls.push([name, args, target?.id ?? null]);
                return true;
            },
            newWindowSize(info) {
                return { w: info.borders.w + 40, h: 50 };
            },
        }),
        {},
        { actions: { stuff: { args: [{ name: "n", type: "number", default: 1 }] } } },
    );
    const a = h.open();
    h.action("stuff", 2);
    assert.deepEqual(calls, [["stuff", [2], a]]);
    assert.deepEqual(h.newWindowSize(undefined, { borders: { w: 4, h: 4 } }), { w: 44, h: 50 });
});

test("an API 1 engine's mode requests and activations are its own", () => {
    const calls = [];
    const h = hostV1((ws) => ({
        ...simple()(ws),
        modeRequested(win, mode, request) {
            calls.push(["mode", win.id, mode, request]);
            ws.setMode(win, mode);
        },
        activate(win) {
            calls.push(["activate", win.id]);
            ws.focus(win);
        },
    }));
    const a = h.open();
    const b = h.open();
    h.requestMode(a, "maximized");
    h.activate(a);
    assert.deepEqual(calls, [
        ["mode", a, "maximized", { maximized: true }],
        ["activate", a],
    ]);
    assert.equal(h.mode(a), "maximized");
    assert.equal(h.focused(), a);
    assert.notEqual(a, b);
});

test("an API 1 engine decides about resizes, which are refused without resizeBegin", () => {
    const refused = hostV1(simple());
    const a = refused.open();
    assert.equal(refused.resize(a, { right: true }, { x: 1, y: 0 }), false);

    const seen = [];
    const h = hostV1((ws) => ({
        ...simple()(ws),
        resizeBegin: (win, resize) => resize.edges.right,
        resizeUpdate: (win, resize) => seen.push(resize.delta.x),
        resizeEnd: () => seen.push("end"),
    }));
    const b = h.open();
    assert.equal(h.resize(b, { left: true }, { x: 1, y: 0 }), false);
    assert.equal(h.resize(b, { right: true }, { x: 5, y: 0 }), true);
    assert.deepEqual(seen, [5, "end"]);
});

test("swipes move the camera for an API 1 engine, which gets gestureEnd", () => {
    const seen = [];
    const h = hostV1((ws) => ({
        ...simple()(ws),
        gestureBegin() {
            seen.push("begin");
            // Camera moves are ignored while the swipe moves it.
            ws.moveCamera({ x: 999 });
        },
        gestureEnd(velocity, projected) {
            seen.push([velocity.x, projected.x > ws.camera.x]);
        },
    }));
    h.swipe(h.workspace(0).id, [10, 20], 100);
    assert.deepEqual(h.camera(), { x: 30, y: 0 });
    assert.deepEqual(seen, ["begin", [100, true]]);
});

test("setHint draws a box that setHint(null) takes away", () => {
    const h = hostV1(
        (ws) => ({
            ...simple()(ws),
            action(name) {
                ws.setHint(name === "hint" ? { x: 1, y: 2, w: 3, h: 4 } : null);
                return true;
            },
        }),
        {},
        { actions: { hint: {}, unhint: {} } },
    );
    h.open();
    const boxes = () => [...h.tree().nodes].filter(([, node]) => node.kind === "box");
    h.action("hint");
    assert.deepEqual(
        boxes().map(([, node]) => node.props.rect),
        [{ x: 1, y: 2, w: 3, h: 4 }],
    );
    h.action("unhint");
    assert.equal(boxes().length, 0);
});

test("tab bars are drawn, and clicking a tab activates its window", () => {
    const activated = [];
    const h = hostV1(
        (ws) => ({
            ...simple()(ws),
            action() {
                ws.setTabBar("tabs", { area: { x: 100, y: 100, w: 200, h: 200 }, windows: ws.windows, active: ws.windows[0] });
                return true;
            },
            activate(win) {
                activated.push(win.id);
            },
        }),
        {},
        { actions: { tabs: {} } },
    );
    const a = h.open();
    const b = h.open();
    h.action("tabs");
    const tabs = [...h.tree().nodes].filter(([, node]) => node.kind === "box" && node.props.interactive);
    assert.equal(tabs.length, 2);
    h.pointer(tabs[1][0], "click");
    assert.deepEqual(activated, [b]);
    assert.notEqual(a, b);

    // A window that's gone leaves the tab bar.
    h.close(b);
    assert.equal([...h.tree().nodes].filter(([, node]) => node.kind === "box").length, 1);
});

test("an API 1 engine's drop target is drawn while a window is dragged", () => {
    const h = hostV1((ws) => ({
        ...simple()(ws),
        dropTarget: (win, pointer) => ({ x: pointer.x, y: pointer.y, w: 5, h: 5 }),
    }));
    const a = h.open();
    const ws = h.workspace(0).id;
    const info = h.windowInfo(h.windows.get(a));
    h.send({ type: "dragenter", workspace: ws, window: info, pointer: { x: 10, y: 20 }, rect: { x: 0, y: 0, w: 5, h: 5 } });
    const boxes = () => [...h.tree().nodes].filter(([, node]) => node.kind === "box");
    assert.deepEqual(boxes()[0][1].props.rect, { x: 10, y: 20, w: 5, h: 5 });
    h.send({ type: "dragleave", workspace: ws });
    assert.equal(boxes().length, 0);
});

test("setActiveInColumn has no effect on an API 1 window", () => {
    let win;
    const h = hostV1((ws) => ({
        windowAdded(w) {
            win = w;
            ws.place(w, { x: 0, y: 0, w: 10, h: 10 });
            ws.setActiveInColumn(w, true);
        },
        windowRemoved() {},
    }));
    h.open();
    assert.equal(win.activeInColumn, null);
});
