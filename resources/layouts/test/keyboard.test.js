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

// The keyboard and the modifiers.

/** The modifiers in `mods` that are held, like `"shift+ctrl"`. */
const held = (mods) =>
    Object.keys(mods)
        .filter((name) => mods[name])
        .join("+");

/**
 * An engine that logs the keyboard events it gets into `log`. Its actions `grab` and `release`
 * take and give back the keyboard, and `grab`'s answers go to `grabs`. Each `keydown` that
 * `prevent` returns true for is prevented.
 */
function keyboardEngine(log, grabs = [], prevent = () => false) {
    return (bento) => {
        for (const type of ["keydown", "keyup"]) {
            bento.addEventListener(type, (e) => {
                log.push([type, e.key, e.text, held(e.modifiers), e.repeat]);
                if (type === "keydown" && prevent(e)) e.preventDefault();
            });
        }
        bento.addEventListener("keyboardrelease", (e) => log.push(["keyboardrelease", e.reason, bento.keyboardGrabbed]));
        bento.addEventListener("modifierschange", (e) =>
            log.push(["modifierschange", held(e.modifiers), held(bento.modifiers)]),
        );
        bento.addEventListener("action", (e) => {
            if (e.name === "grab") grabs.push(bento.grabKeyboard());
            if (e.name === "release") bento.releaseKeyboard();
        });
        bento.exposeState(() => ({ grabbed: bento.keyboardGrabbed, modifiers: { ...bento.modifiers } }));
    };
}

test("an engine gets keys only while it holds the keyboard", () => {
    const log = [];
    const grabs = [];
    const h = host(keyboardEngine(log, grabs));
    assert.equal(h.keyDown("a"), "app");
    assert.equal(h.keyUp("a"), "app");
    assert.deepEqual(log, []);
    assert.equal(h.exposedState().grabbed, false);

    h.action("grab");
    assert.deepEqual(grabs, [true]);
    assert.equal(h.exposedState().grabbed, true);
    assert.equal(h.key("a"), "engine");
    assert.deepEqual(log, [
        ["keydown", "a", "a", "", false],
        ["keyup", "a", "a", "", false],
    ]);
    assert.deepEqual(h.appKeys, [
        ["down", "a"],
        ["up", "a"],
    ]);

    h.action("release");
    log.length = 0;
    assert.equal(h.key("a"), "app");
    assert.deepEqual(log, []);
});

test("keys come with their name, the text they type and the modifiers held", () => {
    const log = [];
    const h = host(keyboardEngine(log));
    h.action("grab");
    h.keyDown("Shift_L");
    h.key("A");
    h.key("Left");
    h.keyDown("Left", { repeat: true });
    h.keyUp("Left");
    h.keyDown("1", { text: "!" });
    h.keyUp("1");
    h.keyUp("Shift_L");
    assert.deepEqual(log, [
        ["keydown", "Shift_L", "", "shift", false],
        ["modifierschange", "shift", "shift"],
        ["keydown", "A", "A", "shift", false],
        ["keyup", "A", "A", "shift", false],
        ["keydown", "Left", "", "shift", false],
        ["keyup", "Left", "", "shift", false],
        ["keydown", "Left", "", "shift", true],
        ["keyup", "Left", "", "shift", false],
        ["keydown", "1", "!", "shift", false],
        ["keyup", "1", "!", "shift", false],
        ["keyup", "Shift_L", "", "", false],
        ["modifierschange", "", ""],
    ]);
});

test("Escape gives the keyboard back, and its release goes to nobody", () => {
    const log = [];
    const h = host(keyboardEngine(log));
    h.action("grab");
    assert.equal(h.keyDown("Escape"), "engine");
    // The engine saw the key, and then that the keyboard went back.
    assert.deepEqual(log, [
        ["keydown", "Escape", "", "", false],
        ["keyboardrelease", "escape", false],
    ]);
    assert.equal(h.keyboard.grabbed, false);
    assert.equal(h.exposedState().grabbed, false);
    // The apps never saw it pressed, so they don't see it released.
    assert.equal(h.keyUp("Escape"), "none");
    assert.deepEqual(h.appKeys, []);
    assert.equal(log.length, 2);
    assert.equal(h.key("b"), "app");
});

test("an engine that prevents Escape keeps the keyboard until it gives it back", () => {
    const log = [];
    const h = host(keyboardEngine(log, [], (e) => e.key === "Escape"));
    h.action("grab");
    assert.equal(h.keyDown("Escape"), "engine");
    assert.equal(h.keyboard.grabbed, true);
    assert.equal(h.keyUp("Escape"), "engine");
    assert.equal(h.key("x"), "engine");
    assert.deepEqual(
        log.map((entry) => entry[0]),
        ["keydown", "keyup", "keydown", "keyup"],
    );

    h.action("release");
    assert.deepEqual(log.at(-1), ["keyboardrelease", "engine", false]);
    assert.equal(h.keyboard.grabbed, false);
});

test("preventing the default of other keys changes nothing", () => {
    const log = [];
    const h = host(keyboardEngine(log, [], () => true));
    h.action("grab");
    h.key("q");
    h.key("Return");
    assert.equal(h.keyboard.grabbed, true);
});

test("releaseKeyboard tells the engine after the call that made it", () => {
    const order = [];
    const h = host((bento) => {
        bento.addEventListener("keyboardrelease", (e) => order.push(["event", e.reason, bento.keyboardGrabbed]));
        bento.addEventListener("action", (e) => {
            if (e.name === "grab") bento.grabKeyboard();
            if (e.name === "release") {
                bento.releaseKeyboard();
                // Releasing what isn't held does nothing.
                bento.releaseKeyboard();
                order.push(["released", bento.keyboardGrabbed]);
            }
        });
    });
    h.action("grab");
    h.action("release");
    assert.deepEqual(order, [
        ["released", false],
        ["event", "engine", false],
    ]);
    h.action("release");
    assert.equal(order.length, 3);
});

test("grabbing twice is one grab", () => {
    const log = [];
    const grabs = [];
    const h = host(keyboardEngine(log, grabs));
    h.action("grab");
    h.action("grab");
    assert.deepEqual(grabs, [true, true]);
    h.action("release");
    h.action("release");
    assert.deepEqual(log, [["keyboardrelease", "engine", false]]);
});

test("binds that can be inhibited don't run while the engine holds the keyboard", () => {
    const log = [];
    const h = host(keyboardEngine(log));
    h.bind("q");
    h.bind("p", { allowInhibiting: false });
    assert.equal(h.key("q"), "bind");
    assert.equal(h.key("p"), "bind");
    assert.deepEqual(log, []);

    h.action("grab");
    // `q` goes to the engine like any other key, and `p` still runs its bind.
    assert.equal(h.key("q"), "engine");
    assert.equal(h.key("p"), "bind");
    assert.deepEqual(log, [
        ["keydown", "q", "q", "", false],
        ["keyup", "q", "q", "", false],
    ]);
    assert.deepEqual(h.ranBinds, ["q", "p", "p"]);
});

test("a key pressed before the grab is released to the apps, and one pressed in it to nobody", () => {
    const log = [];
    const h = host(keyboardEngine(log));
    h.keyDown("a");
    h.action("grab");
    assert.equal(h.keyUp("a"), "app");
    h.keyDown("b");
    h.action("release");
    assert.equal(h.keyUp("b"), "none");
    assert.deepEqual(log, [
        ["keydown", "b", "b", "", false],
        ["keyboardrelease", "engine", false],
    ]);
});

test("a press outside what the engine draws gives the keyboard back", () => {
    const log = [];
    const h = host(keyboardEngine(log));
    h.pressOutsideNodes();
    assert.deepEqual(log, []);
    h.action("grab");
    h.pressOutsideNodes();
    assert.deepEqual(log, [["keyboardrelease", "click", false]]);
    h.pressOutsideNodes();
    assert.equal(log.length, 1);
    assert.equal(h.key("a"), "app");
});

test("the lock screen refuses the keyboard and takes it back", () => {
    const log = [];
    const grabs = [];
    const h = host(keyboardEngine(log, grabs));
    h.lock();
    h.action("grab");
    assert.deepEqual(grabs, [false]);
    assert.equal(h.exposedState().grabbed, false);
    h.lock(false);
    h.action("grab");
    assert.deepEqual(grabs, [false, true]);

    h.lock();
    assert.deepEqual(log, [["keyboardrelease", "lock", false]]);
    assert.equal(h.keyboard.grabbed, false);
    assert.equal(h.key("a"), "app");
});

test("keyboard focus moving to something that isn't the engine's takes the keyboard back", () => {
    const log = [];
    const grabs = [];
    const h = host(keyboardEngine(log, grabs));
    h.action("grab");
    h.focusKeyboard("other");
    assert.deepEqual(log, [["keyboardrelease", "focus", false]]);
    h.action("grab");
    assert.deepEqual(grabs, [true, false]);
    h.focusKeyboard("layout");
    h.action("grab");
    assert.deepEqual(grabs, [true, false, true]);
    assert.equal(h.keyboard.grabbed, true);
});

test("a query can't take the keyboard", () => {
    const answers = [];
    const h = host((bento) => {
        bento.addEventListener("newwindowsize", (e) => {
            answers.push(bento.grabKeyboard());
            e.respondWith({ w: 1, h: 1 });
        });
    });
    h.newWindowSize();
    assert.deepEqual(answers, [false]);
    assert.equal(h.keyboard.grabbed, false);
});

test("reloading the engine ends the grab without telling anyone", () => {
    const log = [];
    const h = host(keyboardEngine(log));
    h.action("grab");
    h.reload();
    assert.equal(h.keyboard.grabbed, false);
    assert.equal(h.exposedState().grabbed, false);
    assert.equal(h.key("a"), "app");
    assert.deepEqual(log, []);
});

test("keys and releases that arrive when the engine holds nothing do nothing", () => {
    const log = [];
    const h = host(keyboardEngine(log));
    const down = { type: "keydown", key: "a", text: "a", modifiers: {}, repeat: false };
    assert.deepEqual(h.send(down), { prevented: false });
    h.send({ ...down, type: "keyup" });
    h.send({ type: "keyboardrelease", reason: "focus" });
    assert.deepEqual(log, []);
});

test("the engine is told when the modifiers held change, grab or not, and can read them", () => {
    const log = [];
    const h = host(keyboardEngine(log));
    assert.deepEqual(h.exposedState().modifiers, { shift: false, ctrl: false, alt: false, super: false });

    h.setModifiers({ super: true });
    assert.deepEqual(log, [["modifierschange", "super", "super"]]);
    assert.equal(h.exposedState().modifiers.super, true);

    // Pressing the key of a modifier that's held changes nothing, and the same set isn't told twice.
    h.setModifiers({ super: true });
    h.keyDown("Super_L");
    assert.equal(log.length, 1);

    h.setModifiers({ super: true, ctrl: true, alt: true });
    assert.deepEqual(log.at(-1), ["modifierschange", "ctrl+alt+super", "ctrl+alt+super"]);
    h.setModifiers({});
    assert.deepEqual(log.at(-1), ["modifierschange", "", ""]);
    // Never keys outside a grab.
    assert.ok(log.every((entry) => entry[0] === "modifierschange"));
});

test("the modifiers held when the engine starts are what it reads", () => {
    const log = [];
    const h = host(keyboardEngine(log));
    h.setModifiers({ shift: true });
    h.reload();
    assert.equal(h.exposedState().modifiers.shift, true);
    h.setModifiers({});
    assert.deepEqual(log.at(-1), ["modifierschange", "", ""]);
});

test("a search takes the keyboard from the engine, and the engine can't take it during one", () => {
    const log = [];
    const h = new TestHost({
        meta: { api: 2, name: "keys-and-search", actions: { "take-keys": {}, search: {} } },
        default(bento) {
            bento.addEventListener("keyboardrelease", (e) => log.push(["release", e.reason]));
            bento.addEventListener("action", (e) => {
                if (e.name === "take-keys") log.push(["take", bento.grabKeyboard()]);
                if (e.name === "search") log.push(["search", bento.search(null) !== null]);
            });
        },
    });
    h.action("take-keys");
    h.action("search");
    h.action("take-keys");
    assert.deepEqual(log, [
        ["take", true],
        ["search", true],
        ["release", "focus"],
        ["take", false],
    ]);
});
