// Tests for what engines can wait for and take: timers, the keyboard, and the modifiers held.
// They use small engines written here, so what's tested is the prelude and the test host.

import assert from "node:assert/strict";
import { test } from "node:test";

import { TestHost } from "./host.js";

/** A host running an API 2 engine made by `create`. */
function host(create, opts = {}) {
    return new TestHost({ meta: { api: 2, name: "test" }, default: create }, opts);
}

// Timers.

test("a timer calls back once, after its delay", () => {
    const calls = [];
    const h = host((bento) => {
        bento.setTimeout(() => calls.push("late"), 100);
        bento.setTimeout(() => calls.push("early"), 40);
    });
    h.advanceTime(39);
    assert.deepEqual(calls, []);
    h.advanceTime(1);
    assert.deepEqual(calls, ["early"]);
    h.advanceTime(1000);
    assert.deepEqual(calls, ["early", "late"]);
    assert.equal(h.timers.size, 0);
});

test("setTimeout returns distinct ids, and clearTimeout cancels", () => {
    const calls = [];
    const h = host((bento) => {
        const a = bento.setTimeout(() => calls.push("a"), 10);
        const b = bento.setTimeout(() => calls.push("b"), 10);
        assert.notEqual(a, b);
        bento.setTimeout(() => calls.push("c"), 10);
        bento.clearTimeout(b);
    });
    h.advanceTime(10);
    assert.deepEqual(calls, ["a", "c"]);
});

test("a timer cleared by a later call never fires, and clearing twice or unknown ids does nothing", () => {
    const calls = [];
    let id;
    const h = host((bento) => {
        id = bento.setTimeout(() => calls.push("fired"), 50);
        bento.addEventListener("action", () => {
            bento.clearTimeout(id);
            bento.clearTimeout(id);
            bento.clearTimeout(12345);
        });
    });
    assert.equal(h.timers.size, 1);
    h.action("cancel");
    assert.equal(h.timers.size, 0);
    h.advanceTime(100);
    assert.deepEqual(calls, []);
});

test("a timer that starts and is cleared in one call never reaches bento", () => {
    const sent = [];
    const h = host((bento) => {
        bento.addEventListener("action", () => {
            bento.clearTimeout(bento.setTimeout(() => {}, 10));
        });
    });
    const call = h.call.bind(h);
    h.call = (...args) => {
        const out = call(...args);
        sent.push(out);
        return out;
    };
    h.action("go");
    assert.ok(sent.every((out) => out.timers === undefined && out.clearedTimers === undefined));
    assert.equal(h.timers.size, 0);
});

test("a timer's callback runs between events, and what it did is applied", () => {
    const order = [];
    let win;
    const h = host((bento) => {
        bento.addEventListener("windowadd", (e) => {
            win = e.window;
            order.push("windowadd");
            e.window.place({ x: 0, y: 0, w: 10, h: 10 });
            bento.setTimeout(() => {
                order.push("timer");
                win.place({ x: 5, y: 6, w: 20, h: 30 });
            }, 10);
        });
    });
    const a = h.open();
    h.advanceTime(10);
    assert.deepEqual(order, ["windowadd", "timer"]);
    assert.deepEqual(h.rect(a), { x: 5, y: 6, w: 20, h: 30 });
});

test("a timer can start another, and a 0 ms timer waits for the next turn", () => {
    const calls = [];
    const h = host((bento) => {
        bento.setTimeout(() => {
            calls.push("first");
            bento.setTimeout(() => calls.push("second"), 5);
        }, 10);
        bento.setTimeout(() => calls.push("now"), 0);
    });
    assert.deepEqual(calls, []);
    h.advanceTime(0);
    assert.deepEqual(calls, ["now"]);
    h.advanceTime(14);
    assert.deepEqual(calls, ["now", "first"]);
    h.advanceTime(1);
    assert.deepEqual(calls, ["now", "first", "second"]);
});

test("a timer that throws is reported, and the others still fire", () => {
    const calls = [];
    const h = host((bento) => {
        bento.setTimeout(() => {
            throw new Error("boom");
        }, 5);
        bento.setTimeout(() => calls.push("ok"), 5);
    });
    assert.throws(() => h.advanceTime(5), /boom/);
    h.advanceTime(0);
    assert.deepEqual(calls, ["ok"]);
});

test("timers end when the engine is reloaded", () => {
    const calls = [];
    let n = 0;
    const h = host((bento) => {
        const mine = ++n;
        bento.setTimeout(() => calls.push(mine), 100);
    });
    h.advanceTime(50);
    h.reload();
    h.advanceTime(1000);
    // The old instance's timer is gone; only the new instance's fires.
    assert.deepEqual(calls, [2]);
});

test("a timer of an old instance that arrives anyway does nothing", () => {
    const calls = [];
    const h = host((bento) => {
        bento.setTimeout(() => calls.push("old"), 100);
    });
    const [[id]] = [...h.timers];
    h.reload({ state: false });
    // The new instance's timer has another id: ids are never reused across instances.
    assert.equal(h.timers.size, 1);
    h.send({ type: "timer", id });
    assert.deepEqual(calls, []);
});

test("setTimeout checks its arguments and bounds the delay and the number of timers", () => {
    const errors = [];
    const h = host((bento) => {
        const attempt = (fn) => {
            try {
                fn();
            } catch (e) {
                errors.push(e.constructor.name);
            }
        };
        attempt(() => bento.setTimeout("not a function", 10));
        attempt(() => bento.setTimeout(() => {}, 3_600_001));
        attempt(() => bento.setTimeout(() => {}, Infinity));
        // Negative and unparsable delays count as 0, like in a browser.
        bento.setTimeout(() => {}, -5);
        bento.setTimeout(() => {}, NaN);
        bento.setTimeout(() => {}, 3_600_000);
        let made = 0;
        for (let i = 0; i < 1000; i++) {
            try {
                bento.setTimeout(() => {}, 1);
                made++;
            } catch (e) {
                errors.push(e.message);
                break;
            }
        }
        bento.exposeState(() => ({ made }));
    });
    assert.deepEqual(errors.slice(0, 3), ["TypeError", "RangeError", "RangeError"]);
    assert.match(errors[3], /timers waiting/);
    // Three timers were already waiting when the limit was hit.
    assert.equal(h.exposedState().made + 3, 128);
    assert.equal(h.timers.size, 128);
});

test("a fired timer makes room for a new one", () => {
    const h = host((bento) => {
        for (let i = 0; i < 128; i++) bento.setTimeout(() => {}, 1);
        bento.addEventListener("action", () => bento.setTimeout(() => {}, 1));
    });
    assert.throws(() => h.action("more"), /timers waiting/);
    h.advanceTime(1);
    h.action("more");
    assert.equal(h.timers.size, 1);
});

test("a timer started in a query isn't started", () => {
    const calls = [];
    const h = host((bento) => {
        bento.addEventListener("newwindowsize", (e) => {
            calls.push(bento.setTimeout(() => calls.push("fired"), 1));
            e.respondWith({ w: 1, h: 1 });
        });
    });
    h.newWindowSize();
    h.advanceTime(10);
    assert.deepEqual(calls, [0]);
});
