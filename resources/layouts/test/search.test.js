// Tests for search: bento's matching and typing as the test host mirrors them, and the objects and
// events the engine gets. They use small engines written here, so what's tested is the prelude and
// the test host.

import assert from "node:assert/strict";
import { test } from "node:test";

import { TestHost } from "./host.js";
import { RANK, SearchField, rank } from "./search.js";

const ACTIONS = {
    all: {},
    group: {},
    workspace: {},
    cancel: {},
    again: {},
    dissolve: {},
    "group-all": {},
};

/** A host running an API 2 engine made by `create`, which can search. */
function host(create, opts = {}) {
    return new TestHost({ meta: { api: 2, name: "test", actions: ACTIONS }, default: create }, opts);
}

/**
 * An engine that places windows side by side, starts searches on request, puts the field in the
 * tree, and records what it's told in `log`, with windows by id.
 */
function searching(log, extra = () => {}) {
    const ids = (windows) => windows.map((win) => win.id);
    return (bento) => {
        bento.addEventListener("windowadd", (e) => {
            const win = e.window;
            win.place({ x: win.workspace.windows.length * 110 - 110, y: 0, w: 100, h: 100 });
            win.focus();
            win.addEventListener("activaterequest", () => log.push(["activate", win.id]));
        });
        const start = (scope) => {
            const search = bento.search(scope);
            if (search === null) {
                log.push(["refused"]);
                return;
            }
            bento.monitors[0].activeWorkspace.tree.append(search.field);
            search.field.set({ rect: { x: 0, y: 200, w: 300, h: 30 }, placeholder: "Search" });
            search.addEventListener("change", () => log.push(["change", search.query, ids(search.matches)]));
            search.addEventListener("end", (e) => log.push(["end", e.picked?.id ?? null]));
        };
        bento.addEventListener("action", (e) => {
            if (e.name === "all") start(null);
            else if (e.name === "group") start(bento.groups[0]);
            else if (e.name === "workspace") start(bento.monitors[0].activeWorkspace);
            else if (e.name === "cancel") bento.searching.cancel();
            else if (e.name === "again") {
                bento.searching.cancel();
                start(null);
            } else if (e.name === "dissolve") bento.groups[0].dissolve();
            else if (e.name === "group-all") bento.createGroup(bento.windows.slice(0, 3));
        });
        extra(bento);
    };
}

// Matching.

test("a query matches by whole words, then prefixes, then substrings, then letters", () => {
    assert.equal(rank("fire", "fire fox", null), RANK.word);
    assert.equal(rank("fire", "mozilla fire", null), RANK.word);
    assert.equal(rank("fire", "firefox", null), RANK.prefix);
    assert.equal(rank("fire", "mozilla firefox", null), RANK.prefix);
    assert.equal(rank("fox", "firefox", null), RANK.substring);
    assert.equal(rank("frfx", "firefox", null), RANK.letters);
    assert.equal(rank("xyz", "firefox", null), null);
    assert.ok(RANK.word > RANK.prefix && RANK.prefix > RANK.substring && RANK.substring > RANK.letters);
    assert.ok(RANK.letters > RANK.any);
});

test("matching ignores case, and no query matches everything", () => {
    assert.equal(rank("FIRE", "fire fox", null), RANK.word);
    assert.equal(rank("fire", "FIRE FOX", null), RANK.word);
    assert.equal(rank("", null, null), RANK.any);
    assert.equal(rank("  ", "anything", null), RANK.any);
});

test("words end at anything that isn't a letter or a digit", () => {
    assert.equal(rank("txt", "notes.txt", null), RANK.word);
    assert.equal(rank("note", "notes.txt", null), RANK.prefix);
    assert.equal(rank("2", "window 2", null), RANK.word);
    assert.equal(rank("2", "window 22", null), RANK.prefix);
    assert.equal(rank("term", "terminal term", null), RANK.word);
    assert.equal(rank("term", "determine terminal", null), RANK.prefix);
});

test("a window ranks as the best of its title and app id", () => {
    assert.equal(rank("term", "a terminal", "term"), RANK.word);
    assert.equal(rank("term", "a terminal", "xterm"), RANK.prefix);
    assert.equal(rank("term", null, "xterm"), RANK.substring);
    assert.equal(rank("term", null, null), null);
});

// Typing.

test("typing inserts at the caret, and backspace and delete remove around it", () => {
    const field = new SearchField();
    field.insert("abcd");
    field.left();
    field.backspace();
    assert.deepEqual([field.text, field.caret], ["abd", 2]);
    field.delete();
    assert.deepEqual([field.text, field.caret], ["ab", 2]);
    field.home();
    field.backspace();
    assert.deepEqual([field.text, field.caret], ["ab", 0]);
});

test("the caret moves over whole characters", () => {
    const field = new SearchField();
    field.insert("é🇫🇷!");
    field.left();
    field.left();
    assert.equal(field.caret, "é".length);
    field.right();
    field.backspace();
    assert.equal(field.text, "é!");
});

test("shift selects, and moving without it collapses the selection to its edge", () => {
    const field = new SearchField();
    field.insert("abcd");
    field.left(true);
    field.left(true);
    assert.equal(field.selected, "cd");
    field.right(true);
    assert.equal(field.selected, "d");
    field.selectAll();
    field.left();
    assert.deepEqual([field.caret, field.anchor], [0, 0]);
    field.selectAll();
    field.right();
    assert.deepEqual([field.caret, field.anchor], [4, 4]);
});

test("typing replaces the selection, and control characters aren't typed", () => {
    const field = new SearchField();
    field.insert("a\nb\tc");
    assert.equal(field.text, "abc");
    field.left();
    field.left(true);
    field.insert("X");
    assert.equal(field.text, "aXc");
    field.selectAll();
    field.insert("\n");
    assert.equal(field.text, "aXc");
});

// The engine's side.

test("an engine searches every window, and the field is a search node it places", () => {
    const log = [];
    let search = null;
    const h = host(
        searching(log, (bento) => {
            bento.addEventListener("action", () => {
                search = bento.searching;
            });
        }),
    );
    const [a, b, c] = [h.open({ title: "Firefox" }), h.open({ title: "Terminal" }), h.open({ title: "Fire fox" })];
    h.action("all");

    assert.equal(search.scope, null);
    assert.equal(search.query, "");
    // Nothing is typed, so every window matches, in the engine's order, and the engine knows that
    // already.
    assert.deepEqual(search.matches.map((win) => win.id), [a, b, c]);
    assert.deepEqual(log, []);

    assert.equal(search.field.kind, "search");
    const field = h.tree().node(search.field.id);
    assert.equal(field.kind, "search");
    assert.deepEqual(field.props, { rect: { x: 0, y: 200, w: 300, h: 30 }, placeholder: "Search" });
    assert.deepEqual(h.search.scope, { kind: "all" });
});

test("one search runs at a time, and the engine sees it as bento.searching", () => {
    const log = [];
    let seen = null;
    const h = host(
        searching(log, (bento) => {
            bento.addEventListener("action", () => {
                seen = [bento.searching !== null, bento.searching?.scope];
            });
        }),
    );
    h.open();
    h.action("all");
    assert.deepEqual(seen, [true, null]);
    h.action("all");
    assert.deepEqual(log, [["refused"]]);
});

test("typing changes the query and the matches, best match first", () => {
    const log = [];
    const h = host(searching(log));
    const [a, , c] = [h.open({ title: "Firefox" }), h.open({ title: "Terminal" }), h.open({ title: "Fire fox" })];
    h.action("all");

    h.type("fire");
    // "Fire fox" has the whole word, "Firefox" only starts with it, and "Terminal" drops out.
    assert.deepEqual(log, [
        ["change", "f", [a, c]],
        ["change", "fi", [a, c]],
        ["change", "fir", [a, c]],
        ["change", "fire", [c, a]],
    ]);

    // Matches that rank the same stay in the engine's order.
    h.key("BackSpace");
    assert.deepEqual(log.at(-1), ["change", "fir", [a, c]]);
    h.type("z");
    assert.deepEqual(log.at(-1), ["change", "firz", []]);
});

test("moving the caret tells the engine nothing", () => {
    const log = [];
    const h = host(searching(log));
    h.open({ title: "Firefox" });
    h.action("all");
    h.type("fire");
    h.key("Left");
    h.key("Right");
    h.key("Home", { shift: true });
    h.key("a", { ctrl: true });
    assert.deepEqual(
        log.map((entry) => entry[1]),
        ["f", "fi", "fir", "fire"],
    );
});

test("Enter ends the search with the best match, which is then asked to activate", () => {
    const log = [];
    const h = host(searching(log));
    const [a, , c] = [h.open({ title: "Firefox" }), h.open({ title: "Terminal" }), h.open({ title: "Fire fox" })];
    h.action("all");
    h.type("firef");
    assert.equal(h.focused(), c);
    const told = log.length;

    h.key("Return");

    assert.equal(h.search, null);
    assert.deepEqual(log.slice(told), [["end", a], ["activate", a]]);
    // Nothing prevented it, so bento focused the window.
    assert.equal(h.focused(), a);
    // bento's field doesn't outlive the search.
    assert.deepEqual(h.tree().node(h.tree().rootId).children, ["w1", "w2", "w3"]);
});

test("Enter with nothing matching picks no window", () => {
    const log = [];
    const h = host(searching(log));
    h.open({ title: "Firefox" });
    const b = h.open({ title: "Terminal" });
    h.action("all");
    h.type("zzz");
    h.key("Return");

    assert.deepEqual(log.at(-1), ["end", null]);
    assert.equal(h.focused(), b);
    assert.equal(log.some((entry) => entry[0] === "activate"), false);
});

test("Escape ends the search without picking", () => {
    const log = [];
    const h = host(searching(log));
    h.open({ title: "Firefox" });
    const b = h.open({ title: "Terminal" });
    h.action("all");
    h.type("fire");
    h.key("Escape");

    assert.equal(h.search, null);
    assert.deepEqual(log.at(-1), ["end", null]);
    assert.equal(h.focused(), b);
    assert.equal(log.some((entry) => entry[0] === "activate"), false);

    // Typing goes nowhere once it's over.
    const told = log.length;
    h.type("x");
    h.key("Return");
    assert.equal(log.length, told);
});

test("the engine can cancel its search, and start another", () => {
    const log = [];
    const h = host(searching(log));
    const a = h.open({ title: "Firefox" });
    h.action("all");
    h.type("fir");
    h.action("again");

    assert.deepEqual(log, [
        ["change", "f", [a]],
        ["change", "fi", [a]],
        ["change", "fir", [a]],
        ["end", null],
    ]);
    // The new search starts with nothing typed, and the old one's text is gone.
    assert.deepEqual(h.search.scope, { kind: "all" });
    assert.equal(h.search.field.text, "");
});

test("cancel removes the field, and bento forgets the search", () => {
    const log = [];
    let search = null;
    const h = host(
        searching(log, (bento) => {
            bento.addEventListener("action", (e) => {
                if (e.name === "all") search = bento.searching;
            });
        }),
    );
    h.open({ title: "Firefox" });
    h.action("all");
    assert.equal(h.tree().has(search.field.id), true);

    h.action("cancel");
    assert.equal(h.search, null);
    assert.equal(h.tree().has(search.field.id), false);
    assert.equal(search.field.parent, null);
    assert.deepEqual(log, [["end", null]]);

    // Cancelling again does nothing.
    search.cancel();
    assert.deepEqual(log, [["end", null]]);
});

test("a search over a group covers its windows", () => {
    const log = [];
    const h = host(searching(log));
    const [a, , c] = [h.open({ title: "Firefox" }), h.open({ title: "Terminal" }), h.open({ title: "Fire fox" })];
    h.open({ title: "Firewall" });
    h.action("group-all");
    h.action("group");
    assert.equal(h.search.scope.kind, "group");

    h.type("fire");
    assert.deepEqual(log.at(-1), ["change", "fire", [c, a]]);
    // bento only reports the windows of the group.
    assert.deepEqual(h.searchInfo().hits.map(([id]) => id), [a, c]);
});

test("a search over a workspace leaves out the windows on others", () => {
    const log = [];
    const h = host(searching(log));
    const a = h.open({ title: "Firefox" });
    const b = h.open({ title: "Firefox private" });
    h.move(b, h.workspace(1).id);
    h.action("workspace");
    h.type("fire");
    assert.deepEqual(log.at(-1), ["change", "fire", [a]]);
});

test("a search ends when its group is gone", () => {
    const log = [];
    const h = host(searching(log));
    h.open({ title: "Firefox" });
    h.open({ title: "Terminal" });
    h.action("group-all");
    h.action("group");
    h.action("dissolve");

    assert.equal(h.search, null);
    assert.deepEqual(log, [["end", null]]);
});

test("a search ends when its workspace is gone", () => {
    const log = [];
    const h = host(searching(log));
    const a = h.open({ title: "Firefox" });
    h.action("workspace");
    h.showWorkspace(h.workspace(1).id);
    h.open({ title: "Terminal" });
    // The first workspace is empty and isn't shown, so bento removes it.
    h.move(a, h.workspace(1).id);

    assert.equal(h.search, null);
    // The window leaving the workspace empties the matches before the workspace goes.
    assert.deepEqual(log, [
        ["change", "", []],
        ["end", null],
    ]);
});

test("matches follow windows that open, close and change their titles", () => {
    const log = [];
    const h = host(searching(log));
    const a = h.open({ title: "Firefox" });
    const b = h.open({ title: "Terminal" });
    h.action("all");
    h.type("term");
    assert.deepEqual(log.at(-1), ["change", "term", [b]]);
    const typed = log.length;

    const c = h.open({ title: "Terminal Two" });
    assert.deepEqual(log.at(-1), ["change", "term", [b, c]]);

    h.setTitle(c, "Notes");
    assert.deepEqual(log.at(-1), ["change", "term", [b]]);

    h.setTitle(a, "A terminal");
    assert.deepEqual(log.at(-1), ["change", "term", [a, b]]);

    h.close(b);
    assert.deepEqual(log.at(-1), ["change", "term", [a]]);
    // Each was told once.
    assert.equal(log.length, typed + 4);
});

test("a search ends when the engine is reloaded, and nobody is told", () => {
    const log = [];
    const h = host(searching(log));
    h.open({ title: "Firefox" });
    h.action("all");
    h.type("f");
    const told = log.length;

    h.reload();
    assert.equal(h.search, null);
    assert.equal(log.length, told);
});

test("the scope of a search must be a group, a workspace or null", () => {
    const h = host((bento) => {
        bento.addEventListener("action", () => bento.search(bento.windows[0]));
    });
    h.open();
    assert.throws(() => h.action("all"), /group, a workspace or null/);
});

test("a search can't start in a query", () => {
    let started = "not asked";
    const h = host((bento) => {
        bento.addEventListener("newwindowsize", (e) => {
            started = bento.search(null);
            e.respondWith({ w: 1, h: 1 });
        });
    });
    h.open();
    h.newWindowSize();
    assert.equal(started, null);
    assert.equal(h.search, null);
});

test("copying and pasting use bento's clipboard", () => {
    const h = host(searching([]));
    h.open({ title: "Firefox" });
    h.action("all");
    h.type("fire");
    h.key("a", { ctrl: true });
    h.key("x", { ctrl: true });
    assert.equal(h.search.field.text, "");
    assert.equal(h.clipboard, "fire");

    h.key("v", { ctrl: true });
    h.key("v", { ctrl: true });
    assert.equal(h.search.field.text, "firefire");
    h.key("a", { ctrl: true });
    h.key("c", { ctrl: true });
    assert.equal(h.clipboard, "firefire");
});

test("a search started as another ends runs", () => {
    let starts = 0;
    const h = host((bento) => {
        const start = () => {
            starts++;
            const search = bento.search(null);
            if (starts === 1) search.addEventListener("end", start);
        };
        bento.addEventListener("action", start);
    });
    h.action("all");
    h.key("Escape");
    assert.equal(starts, 2);
    assert.notEqual(h.search, null);
});

test("binds that can't be inhibited run during a search, and the others don't", () => {
    const h = host(searching([]));
    h.bind("F1", { allowInhibiting: false });
    h.bind("F2", { allowInhibiting: true });
    h.action("all");
    assert.equal(h.key("F1"), "bind");
    assert.equal(h.key("F2"), "search");
    h.type("x");
    assert.equal(h.search.field.text, "x");
});

test("a press outside the field ends the search", () => {
    const log = [];
    const h = host(searching(log));
    h.open({ title: "Firefox" });
    h.action("all");
    h.pressOutsideNodes();
    assert.equal(h.search, null);
    assert.deepEqual(log.at(-1), ["end", null]);
});
