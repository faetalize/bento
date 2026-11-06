// A stand-in for bento, to test layout engines with node:
//
//     import { TestHost } from "./host.js";
//     const host = await TestHost.load("../bento.js", { options: { gravity: true } });
//     const a = host.open();
//     host.action("focus-left");
//     assert.deepEqual(host.rect(a), { x: 16, y: 16, w: 616, h: 688 });
//
// It talks to the engine through the prelude with the same JSON as bento: each call is a `sync`
// with what changed since the engine was last told, and an event. The session has monitors with
// workspaces, which hold windows. Windows behave like simple clients: they take the size they're
// given, within their size limits. Like bento, the host carries out what the engine answers.

import * as prelude from "../prelude/index.js";
import { SearchField, rank } from "./search.js";

const DEFAULT_MONITOR = { size: { w: 1280, h: 720 }, scale: 1, workspaces: 2 };

/** The options written for `name` in `by`, which match names the way bento does: ignoring case. */
function scopedOptions(by, name) {
    if (name === null) return {};
    const key = Object.keys(by).find((other) => other.toLowerCase() === name.toLowerCase());
    return key === undefined ? {} : by[key];
}

function setScoped(by, name, options) {
    const key = Object.keys(by).find((other) => other.toLowerCase() === name.toLowerCase()) ?? name;
    if (options === null) {
        delete by[key];
    } else {
        by[key] = options;
    }
}
const MAX_ROUNDS = 8;
/** Longest wait that bento schedules a timer for, and most timers `advanceTime` fires in one go. */
const MAX_TIMER_DELAY = 3_600_000;
const MAX_TIMERS_FIRED = 10_000;

/** The modifier that each modifier key holds. */
const MODIFIER_KEYS = {
    Shift_L: "shift",
    Shift_R: "shift",
    Control_L: "ctrl",
    Control_R: "ctrl",
    Alt_L: "alt",
    Alt_R: "alt",
    Super_L: "super",
    Super_R: "super",
};

const ALL_DECORATIONS = { border: true, focusRing: true, shadow: true };

export class TestHost {
    /**
     * Loads a layout engine's module and creates it for an empty session.
     *
     * `opts.monitors` lists the monitors, each `{ name, size, scale, workingArea, workspaces,
     * position }`; by default there's one, `"DP-1"`, 1280 × 720 with two workspaces. `opts.config`
     * overrides parts of the workspaces' config, `opts.options` are the engine's options,
     * `opts.outputOptions` and `opts.workspaceOptions` the options that differ per monitor name
     * and per workspace name, and `opts.state` the state the previous instance of the engine
     * saved.
     */
    static async load(path, opts = {}) {
        const url = new URL(path, import.meta.url);
        const module = await import(url.href);
        return new TestHost(module, opts);
    }

    constructor(module, opts = {}) {
        this.meta = JSON.parse(prelude.load(module));
        this.options = opts.options ?? {};
        this.outputOptions = opts.outputOptions ?? {};
        this.workspaceOptions = opts.workspaceOptions ?? {};
        this.config = {
            gaps: 16,
            struts: { left: 0, right: 0, top: 0, bottom: 0 },
            ...(opts.config ?? {}),
        };
        this.nextWindowId = 1;
        this.nextWorkspaceId = 1;
        this.monitors = new Map();
        this.workspaces = new Map();
        this.windows = new Map();
        /** Groups of windows, which bento owns: `{ id, windows, active }`. */
        this.groups = [];
        this.nextGroupId = 1;
        this.focusedMonitor = null;
        /** What the engine was told, by object, as JSON. */
        this.told = { monitors: new Map(), workspaces: new Map(), windows: new Map() };
        this.toldGroups = null;
        this.toldOptions = null;
        /**
         * The running search, which bento owns: `{ scope, field }`, with the scope as the engine
         * asked for it, `{ kind: "all" }`, `{ kind: "workspace", id }` or `{ kind: "group", id }`.
         */
        this.search = null;
        this.toldSearch = null;
        /** bento's clipboard, for copying and pasting in the search field. */
        this.clipboard = "";
        this.trees = new Map();
        this.logs = [];
        this.errors = [];
        this.exposed = null;
        /** Results of cancelable events, in order. */
        this.verdicts = [];
        /** Ops that bento refuses, like a move when the target can't take the window. */
        this.refused = new Set();
        /** The pointer move or resize that the engine began in the last press, until it's taken. */
        this.grab = null;
        /** The clock that timers run by, in milliseconds, and the timers waiting: id to due time. */
        this.now = 0;
        this.timers = new Map();
        /**
         * The keyboard: whether the engine holds it, and who has the focus that it could take,
         * `"layout"`, `"lock"` or `"other"`, like a layer surface.
         */
        this.keyboard = { grabbed: false, owner: "layout" };
        this.toldKeyboardAvailable = null;
        /** Modifiers held, and as the engine was last told. */
        this.modifiers = { shift: false, ctrl: false, alt: false, super: false };
        this.toldModifiers = JSON.stringify(this.modifiers);
        /** Keys pressed while the engine held the keyboard, which the apps never saw pressed, with their text. */
        this.engineKeys = new Map();
        /** bento's own binds by key, `{ allowInhibiting }`, and the keys that ran them or reached apps. */
        this.binds = new Map();
        this.ranBinds = [];
        this.appKeys = [];
        /** Events bento queued for the engine, which it gets when bento flushes. */
        this.queued = [];

        let x = 0;
        for (const given of opts.monitors ?? [{ name: "DP-1" }]) {
            const size = given.size ?? DEFAULT_MONITOR.size;
            this.addMonitor({
                ...DEFAULT_MONITOR,
                position: { x, y: 0 },
                ...given,
                size,
            });
            x += size.w;
        }
        this.focusedMonitor = [...this.monitors.keys()][0] ?? null;

        prelude.setGroupIdSource(() => this.nextGroupId++);
        this.create(opts.state ?? null);
    }

    // The session.

    addMonitor(given) {
        const monitor = {
            name: given.name,
            size: given.size,
            scale: given.scale,
            workingArea: given.workingArea ?? { x: 0, y: 0, w: given.size.w, h: given.size.h },
            position: given.position,
            active: null,
        };
        this.monitors.set(monitor.name, monitor);
        for (let i = 0; i < given.workspaces; i++) {
            this.addWorkspace(monitor.name);
        }
        monitor.active = this.stack(monitor.name)[0].id;
        return monitor;
    }

    addWorkspace(monitor, name = null) {
        const stack = this.stack(monitor);
        const ws = {
            id: this.nextWorkspaceId++,
            name,
            monitor,
            index: stack.length,
            camera: { x: 0, y: 0, zoom: 1 },
            focused: null,
        };
        this.workspaces.set(ws.id, ws);
        this.trees.set(ws.id, new Tree(ws.id));
        return ws;
    }

    /** The workspaces of a monitor, from the top. */
    stack(monitor) {
        return [...this.workspaces.values()]
            .filter((ws) => ws.monitor === monitor)
            .sort((a, b) => a.index - b.index);
    }

    /** The workspace at `index` on a monitor, by default the first one. */
    workspace(index = 0, monitor = this.focusedMonitor) {
        return this.stack(monitor)[index];
    }

    activeWorkspace(monitor = this.focusedMonitor) {
        return this.workspaces.get(this.monitors.get(monitor).active);
    }

    removeWorkspace(id) {
        const ws = this.workspaces.get(id);
        this.workspaces.delete(id);
        this.trees.delete(id);
        this.stack(ws.monitor).forEach((other, i) => {
            other.index = i;
        });
    }

    removeMonitor(name) {
        for (const ws of this.stack(name)) {
            this.workspaces.delete(ws.id);
            this.trees.delete(ws.id);
        }
        this.monitors.delete(name);
        if (this.focusedMonitor === name) {
            this.focusedMonitor = [...this.monitors.keys()][0] ?? null;
        }
        this.flush();
    }

    // What bento tells the engine.

    monitorInfo(monitor) {
        const rect = (m) => ({ ...m.position, w: m.size.w, h: m.size.h });
        const me = rect(monitor);
        const others = [...this.monitors.values()].filter((m) => m !== monitor);
        const near = (pick) => pick(others.map((m) => ({ m, r: rect(m) })))?.m.name ?? null;
        const overlapY = (r) => r.y < me.y + me.h && me.y < r.y + r.h;
        const overlapX = (r) => r.x < me.x + me.w && me.x < r.x + r.w;
        return {
            name: monitor.name,
            size: monitor.size,
            scale: monitor.scale,
            workingArea: monitor.workingArea,
            active: monitor.active,
            left: near((list) => list.filter(({ r }) => overlapY(r) && r.x + r.w <= me.x).at(-1)),
            right: near((list) => list.find(({ r }) => overlapY(r) && me.x + me.w <= r.x)),
            up: near((list) => list.filter(({ r }) => overlapX(r) && r.y + r.h <= me.y).at(-1)),
            down: near((list) => list.find(({ r }) => overlapX(r) && me.y + me.h <= r.y)),
        };
    }

    workspaceInfo(ws) {
        const monitor = this.monitors.get(ws.monitor);
        return {
            id: ws.id,
            name: ws.name,
            monitor: ws.monitor,
            index: ws.index,
            view: { size: monitor.size, workingArea: monitor.workingArea, scale: monitor.scale },
            camera: ws.camera,
            config: this.config,
            options: this.engineOptions(ws.monitor, ws.name),
            focused: ws.focused,
        };
    }

    windowInfo(win) {
        return {
            id: win.id,
            appId: win.appId,
            title: win.title,
            workspace: win.workspace,
            size: win.size,
            minSize: win.minSize,
            maxSize: win.maxSize,
            // The border isn't there for a window that bento draws none around.
            borders: win.decorations.border ? win.borders : { w: 0, h: 0 },
            decorations: win.decorations,
            mode: win.mode,
            urgent: win.urgent,
            rules: win.rules,
            data: win.data,
        };
    }

    /** What changed since the engine was last told. */
    sync() {
        const sync = { monitors: [], removedMonitors: [], workspaces: [], removedWorkspaces: [], windows: [] };
        const diff = (told, now, key, push) => {
            for (const [id, info] of now) {
                if (JSON.stringify(told.get(id)) !== JSON.stringify(info)) {
                    push(info);
                }
                told.set(id, info);
            }
            for (const id of [...told.keys()]) {
                if (!now.has(id)) {
                    key(id);
                    told.delete(id);
                }
            }
        };
        diff(
            this.told.monitors,
            new Map([...this.monitors].map(([name, m]) => [name, this.monitorInfo(m)])),
            (name) => sync.removedMonitors.push(name),
            (info) => sync.monitors.push(info),
        );
        diff(
            this.told.workspaces,
            new Map([...this.workspaces].map(([id, ws]) => [id, this.workspaceInfo(ws)])),
            (id) => sync.removedWorkspaces.push(id),
            (info) => sync.workspaces.push(info),
        );
        // Windows come and go with events, not with the sync.
        for (const win of this.windows.values()) {
            const info = this.windowInfo(win);
            const told = this.told.windows.get(win.id);
            if (told !== undefined && JSON.stringify(told) !== JSON.stringify(info)) {
                sync.windows.push(info);
            }
            if (told !== undefined) {
                this.told.windows.set(win.id, info);
            }
        }
        const groups = this.groups.map((group) => ({ ...group, windows: [...group.windows] }));
        if (JSON.stringify(this.toldGroups) !== JSON.stringify(groups)) {
            sync.groups = groups;
            this.toldGroups = groups;
        }
        const search = this.searchInfo();
        if (JSON.stringify(this.toldSearch) !== JSON.stringify(search)) {
            if (search !== null) sync.search = search;
            this.toldSearch = search;
        }
        sync.focusedMonitor = this.focusedMonitor;
        // Like bento, the keyboard has one owner: a search that has the keys keeps them.
        const available = this.keyboard.owner === "layout" && this.search === null;
        if (this.toldKeyboardAvailable !== available) {
            sync.keyboardAvailable = available;
            this.toldKeyboardAvailable = available;
        }
        const options = this.engineOptions();
        if (JSON.stringify(this.toldOptions) !== JSON.stringify(options)) {
            sync.options = options;
            this.toldOptions = options;
        }
        return sync;
    }

    isEmptySync(sync) {
        return (
            sync.monitors.length === 0 &&
            sync.removedMonitors.length === 0 &&
            sync.workspaces.length === 0 &&
            sync.removedWorkspaces.length === 0 &&
            sync.windows.length === 0 &&
            sync.groups === undefined &&
            sync.search === undefined &&
            sync.options === undefined &&
            sync.keyboardAvailable === undefined &&
            sync.focusedMonitor === this.toldFocusedMonitor
        );
    }

    create(state) {
        const sync = this.sync();
        this.toldFocusedMonitor = this.focusedMonitor;
        const windows = [...this.windows.values()].map((win) => ({ ...this.windowInfo(win), rect: win.rect }));
        for (const win of windows) {
            this.told.windows.set(win.id, this.windowInfo(this.windows.get(win.id)));
        }
        this.call(
            {
                sync,
                event: {
                    type: "create",
                    options: this.engineOptions(),
                    savedState: state,
                    windows,
                    focused: this.focused(),
                    modifiers: { ...this.modifiers },
                },
            },
            true,
        );
        this.flush();
    }

    /** Sends an event, and carries out what the engine did. Returns the call's result. */
    send(event) {
        this.flush();
        return this.sendNow(event);
    }

    /**
     * Sends an event with what changed since the engine was last told, like bento sends what an
     * event changed along with it.
     */
    sendNow(event) {
        const sync = this.sync();
        this.toldFocusedMonitor = this.focusedMonitor;
        // Like bento, monitors and workspaces that went away go after the event, which can name
        // them.
        const removals = {
            monitors: [],
            removedMonitors: sync.removedMonitors,
            workspaces: [],
            removedWorkspaces: sync.removedWorkspaces,
            windows: [],
            focusedMonitor: sync.focusedMonitor,
        };
        sync.removedMonitors = [];
        sync.removedWorkspaces = [];
        const out = this.call({ sync, event });
        if (removals.removedMonitors.length > 0 || removals.removedWorkspaces.length > 0) {
            this.call({ sync: removals, event: { type: "sync" } });
        }
        this.flush();
        return out.result;
    }

    /** Tells the engine about what changed because of what it did, until it settles. */
    flush() {
        while (this.queued.length > 0) {
            this.sendNow(this.queued.shift());
        }
        for (let round = 0; round < MAX_ROUNDS; round++) {
            this.settleSizes();
            const searchEnded = this.refreshSearch();
            const sync = this.sync();
            if (!searchEnded && this.isEmptySync(sync)) {
                return;
            }
            this.toldFocusedMonitor = this.focusedMonitor;
            // Like bento, a search that lost its scope ends with the sync that follows.
            const event = searchEnded ? { type: "searchend", pick: false } : { type: "sync" };
            this.call({ sync, event });
        }
    }

    call(call, creating = false) {
        const out = JSON.parse(prelude.handle(JSON.stringify(call)));
        this.logs.push(...out.logs);
        if (out.error !== null) {
            this.errors.push(out.error);
            throw new Error(`layout engine error: ${out.error}`);
        }
        this.apply(out, creating);
        return out;
    }

    // What the engine asks of bento.

    apply(out) {
        if ((out.ops ?? []).length > 0) {
            // Like bento, tell the engine about groups again after it changed some.
            this.toldGroups = null;
        }
        for (const op of out.ops ?? []) {
            if (this.refused.has(op.op)) {
                // Like bento, tell the engine everything again, which undoes what it did.
                for (const map of [this.told.windows, this.told.workspaces]) {
                    for (const key of map.keys()) map.set(key, null);
                }
                continue;
            }
            switch (op.op) {
                case "show":
                    this.monitors.get(this.workspaces.get(op.workspace).monitor).active = op.workspace;
                    break;
                case "moveWorkspace":
                    this.moveWorkspace(op.workspace, op.monitor, op.index);
                    break;
                case "rename":
                    this.workspaces.get(op.workspace).name = op.name;
                    break;
                case "moveWindow":
                    this.moveWindow(op.window, op.workspace);
                    break;
                case "createGroup":
                    this.createGroup(op.id, op.windows, op.active);
                    break;
                case "groupAdd":
                    this.groupAdd(op.group, op.window);
                    break;
                case "groupRemove":
                    this.leaveGroup(op.window);
                    break;
                case "groupActivate":
                    this.groups.find((group) => group.id === op.group).active = op.window;
                    break;
                case "dissolveGroup":
                    this.groups = this.groups.filter((group) => group.id !== op.group);
                    break;
                case "search":
                    // The search takes the keys, so the engine stops holding the keyboard.
                    if (this.keyboard.grabbed) {
                        this.keyboard.grabbed = false;
                        this.queued.push({ type: "keyboardrelease", reason: "focus" });
                    }
                    this.search = { scope: op.scope, field: new SearchField() };
                    break;
                case "endSearch":
                    this.search = null;
                    break;
                default:
                    throw new Error(`unknown op: ${op.op}`);
            }
        }
        for (const [id, decorations] of out.decorations ?? []) {
            this.windows.get(id).decorations = { ...decorations };
        }
        if (out.grab !== undefined) {
            this.grab = out.grab;
        }
        for (const change of out.windows ?? []) {
            const win = this.windows.get(change.id);
            if (win === undefined) continue;
            if (change.rect !== null) win.rect = change.rect;
            win.visible = change.visible;
            win.mode = change.mode;
            win.animate = change.animate;
        }
        for (const op of out.tree ?? []) {
            this.applyTreeOp(op);
        }
        for (const [id, data] of out.data ?? []) {
            this.windows.get(id).data = data;
        }
        for (const [id, focused] of out.focus ?? []) {
            this.workspaces.get(id).focused = focused;
        }
        for (const camera of out.cameras ?? []) {
            this.workspaces.get(camera.workspace).camera = { x: camera.x, y: camera.y, zoom: camera.zoom };
            this.lastCamera = camera;
        }
        if (out.monitorFocus !== undefined) {
            this.focusedMonitor = out.monitorFocus;
        }
        if (out.stateChanged) {
            this.stateChanges = (this.stateChanges ?? 0) + 1;
        }
        for (const timer of out.timers ?? []) {
            this.timers.set(timer.id, this.now + Math.min(timer.ms, MAX_TIMER_DELAY));
        }
        for (const id of out.clearedTimers ?? []) {
            this.timers.delete(id);
        }
        if (out.keyboard === true) {
            if (this.keyboard.owner === "layout" && this.search === null) {
                this.keyboard.grabbed = true;
            } else {
                // It can't be taken after all, which the engine hears like a release.
                this.queued.push({ type: "keyboardrelease", reason: this.keyboard.owner === "lock" ? "lock" : "focus" });
            }
        } else if (out.keyboard === false) {
            this.keyboard.grabbed = false;
        }
    }

    // The keyboard.

    /** Takes the keyboard from the engine, which hears why. Returns whether it held it. */
    releaseKeyboard(reason) {
        if (!this.keyboard.grabbed) return false;
        this.keyboard.grabbed = false;
        this.queued.push({ type: "keyboardrelease", reason });
        return true;
    }

    /** Moves the keyboard focus to `owner`: the layout, the `"lock"` screen, or `"other"`. */
    focusKeyboard(owner) {
        this.keyboard.owner = owner;
        if (owner === "lock") this.releaseKeyboard("lock");
        else if (owner === "other") this.releaseKeyboard("focus");
        this.flush();
    }

    /** The session locks, or unlocks. */
    lock(locked = true) {
        this.focusKeyboard(locked ? "lock" : "layout");
    }

    /**
     * One of bento's own binds on `key`. Like binds that apps can inhibit, it doesn't run while
     * the engine holds the keyboard, unless `allowInhibiting` is false.
     */
    bind(key, { allowInhibiting = true } = {}) {
        this.binds.set(key, { allowInhibiting });
    }

    /**
     * Presses a key, with `opts.text` it types (by default the key itself if it's one character,
     * else nothing) and `opts.repeat`. A modifier key changes the modifiers held. Returns where
     * the key went: `"bind"`, `"engine"` or `"app"`.
     */
    keyDown(key, opts = {}) {
        const modifier = MODIFIER_KEYS[key];
        const changes = modifier !== undefined && !this.modifiers[modifier];
        if (changes) this.modifiers[modifier] = true;
        const bind = this.binds.get(key);
        let went;
        if (bind !== undefined && !(this.keyboard.grabbed && bind.allowInhibiting)) {
            this.ranBinds.push(key);
            went = "bind";
        } else if (this.keyboard.grabbed) {
            went = "engine";
            const text = opts.text ?? (key.length === 1 ? key : "");
            this.engineKeys.set(key, text);
            const verdict = this.send({
                type: "keydown",
                key,
                text,
                modifiers: { ...this.modifiers },
                repeat: opts.repeat ?? false,
            });
            if (key === "Escape" && !verdict.prevented) {
                this.releaseKeyboard("escape");
                this.flush();
            }
        } else {
            this.appKeys.push(["down", key]);
            went = "app";
        }
        if (changes) this.sendModifiers();
        return went;
    }

    /**
     * Releases a key. A key that the engine got the press of is released to it while it still
     * holds the keyboard (`"engine"`, else `"none"`), and one pressed before that goes to the apps.
     */
    keyUp(key) {
        const modifier = MODIFIER_KEYS[key];
        const changes = modifier !== undefined && this.modifiers[modifier];
        if (changes) this.modifiers[modifier] = false;
        let went;
        const text = this.engineKeys.get(key);
        if (this.engineKeys.delete(key)) {
            // The apps never saw the press, so they don't get the release either.
            went = this.keyboard.grabbed ? "engine" : "none";
            if (this.keyboard.grabbed) {
                this.send({
                    type: "keyup",
                    key,
                    text,
                    modifiers: { ...this.modifiers },
                    repeat: false,
                });
            }
        } else if (this.binds.has(key) && this.ranBinds.includes(key)) {
            went = "bind";
        } else {
            this.appKeys.push(["up", key]);
            went = "app";
        }
        if (changes) this.sendModifiers();
        return went;
    }

    /** Presses and releases a key, and returns where the press went. */
    key(key, opts = {}) {
        // Like bento, a running search takes the keys.
        if (this.search !== null) {
            return this.searchKey(key, opts);
        }
        const went = this.keyDown(key, opts);
        this.keyUp(key);
        return went;
    }

    /** Tells the engine the modifiers held if they changed since it was last told. */
    sendModifiers() {
        const now = JSON.stringify(this.modifiers);
        if (now !== this.toldModifiers) {
            this.toldModifiers = now;
            this.send({ type: "modifierschange", modifiers: { ...this.modifiers } });
        }
    }

    /** Holds exactly these modifiers, like keys that aren't shown here. Missing ones are released. */
    setModifiers(held) {
        this.modifiers = { shift: false, ctrl: false, alt: false, super: false, ...held };
        this.sendModifiers();
    }

    /** A pointer press that no node of the engine's takes. It ends the engine's hold on the keyboard. */
    pressOutsideNodes() {
        this.releaseKeyboard("click");
        this.flush();
    }

    /**
     * Lets `ms` milliseconds pass, sending the engine a `timer` event for each timer that comes
     * due, earliest first, each between other events like in bento.
     */
    advanceTime(ms) {
        const until = this.now + ms;
        for (let fired = 0; ; fired++) {
            const next = [...this.timers].filter(([, due]) => due <= until).sort((a, b) => a[1] - b[1] || a[0] - b[0])[0];
            if (next === undefined) {
                break;
            }
            if (fired >= MAX_TIMERS_FIRED) {
                throw new Error(`more than ${MAX_TIMERS_FIRED} timers fired in ${ms} ms`);
            }
            const [id, due] = next;
            this.now = Math.max(this.now, due);
            this.timers.delete(id);
            this.send({ type: "timer", id });
        }
        this.now = until;
    }

    moveWorkspace(id, monitor, index) {
        const ws = this.workspaces.get(id);
        const from = ws.monitor;
        const stack = this.stack(monitor).filter((other) => other !== ws);
        stack.splice(index ?? stack.length, 0, ws);
        ws.monitor = monitor;
        stack.forEach((other, i) => {
            other.index = i;
        });
        this.stack(from).forEach((other, i) => {
            other.index = i;
        });
        const source = this.monitors.get(from);
        if (source.active === id && from !== monitor) {
            source.active = this.stack(from)[0].id;
        }
    }

    /** Moves a window to another workspace, like bento does for the engine, without an event. */
    moveWindow(id, workspace) {
        const win = this.windows.get(id);
        const from = this.workspaces.get(win.workspace);
        if (from.focused === id) from.focused = null;
        const subtree = this.trees.get(from.id).take(`w${id}`);
        win.workspace = workspace;
        this.trees.get(workspace).put(subtree);
    }

    applyTreeOp(op) {
        const find = (id) => [...this.trees.values()].find((tree) => tree.has(id));
        if (op.op === "create") {
            this.pending ??= new Map();
            this.pending.set(op.id, op);
        } else if (op.op === "append") {
            const home = find(op.parent);
            const from = find(op.child);
            const create = this.pending?.get(op.child);
            if (create !== undefined) {
                this.pending.delete(op.child);
                home.create(create);
            } else if (from !== undefined && from !== home) {
                home.put(from.take(op.child), op.parent, op.before);
                return;
            }
            home.append(op.parent, op.child, op.before);
        } else {
            find(op.id)?.apply(op);
        }
    }

    // Groups.

    /** Makes group `id` of `windows`, which leave the groups they're in. */
    createGroup(id, windows, active) {
        const members = [...new Set(windows)];
        if (members.length < 2) return;
        members.forEach((window) => this.leaveGroup(window));
        this.groups.push({ id, windows: members, active: members.includes(active) ? active : members[0] });
    }

    groupAdd(id, window) {
        const group = this.groups.find((group) => group.id === id);
        if (group === undefined || group.windows.includes(window)) return;
        this.leaveGroup(window);
        group.windows.push(window);
    }

    /**
     * Takes a window out of its group, which is gone when one window is left. When the window was
     * the active one, the window that takes its place in the order is.
     */
    leaveGroup(window) {
        const group = this.groups.find((group) => group.windows.includes(window));
        if (group === undefined) return;
        const pos = group.windows.indexOf(window);
        group.windows.splice(pos, 1);
        if (group.windows.length < 2) {
            this.groups = this.groups.filter((other) => other !== group);
        } else if (group.active === window) {
            group.active = group.windows[Math.min(pos, group.windows.length - 1)];
        }
    }

    groupOf(window) {
        return this.groups.find((group) => group.windows.includes(window)) ?? null;
    }

    /**
     * bento's `group-window-left` and its siblings: the focused window joins the group of the
     * window nearest to it in `direction` on its workspace, by where the engine put them, or the
     * two make a group. It's the group's active window.
     */
    groupWindow(direction) {
        const focused = this.focused();
        if (focused === null) return;
        const win = this.windows.get(focused);
        if (!win.rect) return;
        const center = (r) => ({ x: r.x + r.w / 2, y: r.y + r.h / 2 });
        const from = center(win.rect);
        const side = {
            left: (c) => c.x < from.x,
            right: (c) => c.x > from.x,
            up: (c) => c.y < from.y,
            down: (c) => c.y > from.y,
        }[direction];
        const gap = (a, b) => Math.max(0, b - a);
        const candidates = [...this.windows.values()]
            .filter((other) => other.id !== focused && other.workspace === win.workspace)
            .filter((other) => other.visible && other.rect && side(center(other.rect)))
            .map((other) => {
                const r = other.rect;
                const w = win.rect;
                const gx = Math.max(gap(w.x + w.w, r.x), gap(r.x + r.w, w.x));
                const gy = Math.max(gap(w.y + w.h, r.y), gap(r.y + r.h, w.y));
                const c = center(r);
                return { id: other.id, gaps: gx * gx + gy * gy, centres: (c.x - from.x) ** 2 + (c.y - from.y) ** 2 };
            })
            .sort((a, b) => a.gaps - b.gaps || a.centres - b.centres || a.id - b.id);
        if (candidates.length === 0) return;

        const target = candidates[0].id;
        const group = this.groupOf(target);
        if (group === null) {
            this.createGroup(this.nextGroupId++, [target, focused], focused);
        } else {
            this.groupAdd(group.id, focused);
            group.active = focused;
        }
        this.flush();
    }

    /** Groups windows on bento's side, the way its native actions do, with `active` active. */
    groupWindows(windows, active) {
        this.createGroup(this.nextGroupId++, windows, active);
        this.flush();
    }

    /** bento's `ungroup-window`: the focused window leaves its group. */
    ungroupWindow() {
        const focused = this.focused();
        if (focused !== null) this.leaveGroup(focused);
        this.flush();
    }

    // Search.

    /** The windows a search over `scope` covers, or null if there's no such group or workspace. */
    searchWindows(scope) {
        switch (scope.kind) {
            case "workspace":
                if (!this.workspaces.has(scope.id)) return null;
                return [...this.windows.values()].filter((win) => win.workspace === scope.id);
            case "group": {
                const group = this.groups.find((other) => other.id === scope.id);
                return group === undefined ? null : group.windows.map((id) => this.windows.get(id));
            }
            default:
                return [...this.windows.values()];
        }
    }

    /** What the engine is told about the search: the query and the windows that match it. */
    searchInfo() {
        const windows = this.search === null ? null : this.searchWindows(this.search.scope);
        if (windows === null) return null;
        const query = this.search.field.text;
        const hits = windows
            .map((win) => [win.id, rank(query, win.title, win.appId)])
            .filter(([, matched]) => matched !== null);
        return { query, hits };
    }

    /** Ends a search whose scope is gone, and returns whether there was one. */
    refreshSearch() {
        if (this.search === null || this.searchWindows(this.search.scope) !== null) {
            return false;
        }
        this.search = null;
        return true;
    }

    /** Types text in the search's field, a character at a time. */
    type(text) {
        for (const c of text) {
            this.editSearch((field) => field.insert(c));
        }
    }

    /**
     * Presses a key while the search runs: `"Return"` picks the best match and `"Escape"` cancels,
     * `"BackSpace"`, `"Delete"`, `"Left"`, `"Right"`, `"Home"` and `"End"` edit and move, with
     * `shift` selecting; with `ctrl`, `"a"` selects all and `"c"`, `"x"` and `"v"` use the
     * clipboard.
     */
    searchKey(name, { shift = false, ctrl = false } = {}) {
        if (ctrl) {
            const field = this.search.field;
            switch (name) {
                case "a":
                    return this.editSearch((f) => f.selectAll());
                case "c":
                    this.clipboard = field.selected ?? this.clipboard;
                    return;
                case "x":
                    if (field.selected !== null) {
                        this.clipboard = field.selected;
                        this.editSearch((f) => f.backspace());
                    }
                    return;
                case "v":
                    return this.editSearch((f) => f.insert(this.clipboard));
                default:
                    return;
            }
        }
        switch (name) {
            case "Return":
                return this.endSearch(true);
            case "Escape":
                return this.endSearch(false);
            case "BackSpace":
                return this.editSearch((f) => f.backspace());
            case "Delete":
                return this.editSearch((f) => f.delete());
            case "Left":
                return this.editSearch((f) => f.left(shift));
            case "Right":
                return this.editSearch((f) => f.right(shift));
            case "Home":
                return this.editSearch((f) => f.home(shift));
            case "End":
                return this.editSearch((f) => f.end(shift));
            default:
                throw new Error(`unknown key: ${name}`);
        }
    }

    editSearch(edit) {
        if (this.search === null) return;
        edit(this.search.field);
        this.flush();
    }

    /**
     * Ends the search like bento does for Enter, with `pick`, and Escape: the engine hears the
     * latest matches and then that it ended, and bento activates the window it picked.
     */
    endSearch(pick) {
        if (this.search === null) return;
        const result = this.send({ type: "searchend", pick });
        this.search = null;
        if (pick && result.picked !== null) {
            this.activate(result.picked);
        }
        this.flush();
    }

    // Windows.

    /** The window that has keyboard focus. */
    focused() {
        if (this.focusedMonitor === null) return null;
        return this.activeWorkspace().focused;
    }

    /**
     * Opens a window on the active workspace of the focused monitor, or on `opts.workspace`. `opts`
     * can set `minSize`, `maxSize`, `borders`, `appId`, `title`, `rules`, `mode`, `activate`
     * (default true), `reason`, `rect`, `pointer` and `nextTo`.
     */
    open(opts = {}) {
        const id = this.nextWindowId++;
        const ws = opts.workspace ?? this.activeWorkspace().id;
        const win = {
            id,
            appId: opts.appId ?? null,
            title: opts.title ?? null,
            workspace: ws,
            size: { w: 0, h: 0 },
            minSize: opts.minSize ?? { w: 0, h: 0 },
            maxSize: opts.maxSize ?? { w: 0, h: 0 },
            borders: opts.borders ?? { w: 0, h: 0 },
            decorations: { ...ALL_DECORATIONS },
            mode: opts.mode ?? "normal",
            urgent: false,
            rules: opts.rules ?? {},
            data: null,
            rect: null,
            visible: true,
        };
        this.windows.set(id, win);
        this.trees.get(ws).add(`w${id}`);
        this.flushBeforeAdd();
        const info = this.windowInfo(win);
        this.told.windows.set(id, info);
        this.send({
            type: "windowadd",
            window: info,
            how: {
                reason: opts.reason ?? "open",
                activate: opts.activate ?? true,
                rect: opts.rect ?? null,
                pointer: opts.pointer ?? null,
                nextTo: opts.nextTo ?? null,
            },
        });
        return id;
    }

    flushBeforeAdd() {
        // Workspaces and monitors the new window needs have to be known before it is.
        const sync = this.sync();
        if (!this.isEmptySync(sync)) {
            this.toldFocusedMonitor = this.focusedMonitor;
            this.call({ sync, event: { type: "sync" } });
        }
    }

    close(id, reason = "close") {
        this.flush();
        const win = this.windows.get(id);
        // What the engine turned off around a window that leaves it comes back on.
        win.decorations = { ...ALL_DECORATIONS };
        this.leaveGroup(id);
        this.windows.delete(id);
        this.told.windows.delete(id);
        this.trees.get(win.workspace).remove(`w${id}`);
        const ws = this.workspaces.get(win.workspace);
        if (ws.focused === id) ws.focused = null;
        this.cleanUpWorkspaces();
        this.sendNow({ type: "windowremove", id, reason });
    }

    /** Moves a window to another workspace like one of bento's own actions does. */
    move(id, workspace, activate = false) {
        this.flush();
        const from = this.windows.get(id).workspace;
        this.moveWindow(id, workspace);
        // Like bento, the event moves the window, not the sync.
        this.told.windows.set(id, this.windowInfo(this.windows.get(id)));
        this.cleanUpWorkspaces();
        this.sendNow({ type: "windowmove", id, from, to: workspace, activate });
    }

    /**
     * Removes workspaces that bento would: empty, unnamed and not shown ones, except the last of
     * each monitor's stack.
     */
    cleanUpWorkspaces() {
        for (const ws of [...this.workspaces.values()]) {
            const stack = this.stack(ws.monitor);
            const empty = ![...this.windows.values()].some((win) => win.workspace === ws.id);
            const shown = this.monitors.get(ws.monitor).active === ws.id;
            if (empty && !shown && ws.name === null && stack[stack.length - 1] !== ws) {
                this.removeWorkspace(ws.id);
            }
        }
    }

    /** Runs an action: a bind in the engine's file does. `args` are as the engine declared them. */
    action(name, ...args) {
        this.send({ type: "action", name, args, window: this.focused() });
    }

    actionOn(id, name, ...args) {
        this.send({ type: "action", name, args, window: id });
    }

    /** Something asks for focus on a window. Without a listener that prevents it, bento does it. */
    activate(id) {
        const verdict = this.send({ type: "activaterequest", id });
        this.verdicts.push(verdict);
        if (!verdict.prevented) {
            const win = this.windows.get(id);
            const ws = this.workspaces.get(win.workspace);
            this.monitors.get(ws.monitor).active = ws.id;
            this.focusedMonitor = ws.monitor;
            ws.focused = id;
        }
        this.flush();
    }

    /**
     * Asks for a sizing mode like bento does: `request` is `{ fullscreen: bool }` or
     * `{ maximized: bool }`; `mode` is what results, by default from the request.
     */
    requestMode(id, mode, request) {
        if (request === undefined) {
            request = mode === "maximized" ? { maximized: true } : { fullscreen: mode === "fullscreen" };
        }
        this.send({ type: "moderequest", id, mode, request });
    }

    /** Changes a window's size limits, like a client committing new ones. */
    setLimits(id, { minSize, maxSize }) {
        const win = this.windows.get(id);
        if (minSize) win.minSize = minSize;
        if (maxSize) win.maxSize = maxSize;
        this.flush();
    }

    setTitle(id, title) {
        this.windows.get(id).title = title;
        this.flush();
    }

    resizeMonitor(name, { size, workingArea, scale }) {
        const monitor = this.monitors.get(name);
        if (size) monitor.size = size;
        monitor.workingArea = workingArea ?? { x: 0, y: 0, w: monitor.size.w, h: monitor.size.h };
        if (scale) monitor.scale = scale;
        this.flush();
    }

    setOptions(options) {
        this.options = options;
        this.flush();
    }

    /** Sets the options of the workspaces on a monitor, or with `null` takes them away. */
    setOutputOptions(monitor, options) {
        setScoped(this.outputOptions, monitor, options);
        this.flush();
    }

    /** Sets the options of the workspaces with a name, or with `null` takes them away. */
    setWorkspaceOptions(name, options) {
        setScoped(this.workspaceOptions, name, options);
        this.flush();
    }

    /**
     * The engine's options as bento sends them, written as in the .kdl file, with defaults filled
     * in and sizes turned into presets. Without arguments they are the top-level options. For a
     * workspace they are those, then its monitor's `outputOptions` and then its own
     * `workspaceOptions` by name, a later one replacing an earlier one option by option.
     */
    engineOptions(monitor = null, workspace = null) {
        const preset = (value) => {
            if (typeof value !== "string") return value;
            return value.endsWith("%")
                ? { proportion: Number(value.slice(0, -1)) / 100 }
                : { fixed: Number(value) };
        };
        const written = {
            ...this.options,
            ...scopedOptions(this.outputOptions, monitor),
            ...scopedOptions(this.workspaceOptions, workspace),
        };
        const options = {};
        for (const spec of this.meta.options) {
            let value = spec.name in written ? written[spec.name] : spec.default;
            if (spec.type === "size") {
                value = spec.list ? value.map(preset) : preset(value);
            }
            options[spec.name] = value;
        }
        return options;
    }

    /** Switches a monitor to a workspace, like bento's own actions do. */
    showWorkspace(workspace) {
        const ws = this.workspaces.get(workspace);
        this.monitors.get(ws.monitor).active = workspace;
        this.focusedMonitor = ws.monitor;
        this.flush();
    }

    /** Tells the engine about a swipe along a workspace: `deltas` are the finger movements. */
    swipe(workspace, deltas, velocity = 0) {
        const event = (type, delta, velocity = 0) => ({ type, workspace, delta, velocity, touchpad: true });
        this.send(event("swipebegin", 0));
        for (const delta of deltas) {
            this.send(event("swipeupdate", delta));
        }
        this.send(event("swipeend", 0, velocity));
    }

    /** Drags a window over a workspace: `positions` are where its pointer goes, then drops it. */
    drag(id, workspace, positions, drop = true) {
        const win = this.windows.get(id);
        const from = win.workspace;
        const rect = (pointer) => ({ x: pointer.x - 10, y: pointer.y - 10, w: 20, h: 20 });
        this.close(id, "drag");
        const info = this.windowInfo(win);
        this.windows.set(id, win);
        positions.forEach((pointer, i) => {
            this.send({
                type: i === 0 ? "dragenter" : "dragmove",
                workspace,
                window: info,
                pointer,
                rect: rect(pointer),
            });
        });
        this.send({ type: "dragleave", workspace });
        if (drop) {
            const pointer = positions.at(-1);
            win.workspace = workspace;
            this.trees.get(workspace).add(`w${id}`);
            this.told.windows.set(id, this.windowInfo(win));
            this.send({
                type: "windowadd",
                window: this.windowInfo(win),
                how: { reason: "drop", activate: true, rect: rect(pointer), pointer, nextTo: null },
            });
        } else {
            this.windows.delete(id);
        }
        return from;
    }

    /** Resizes a window with the pointer; returns whether the engine allowed it. */
    resize(id, edges, delta) {
        const edge = { left: false, right: false, top: false, bottom: false, ...edges };
        const verdict = this.send({
            type: "resizebegin",
            id,
            edges: edge,
            delta: { x: 0, y: 0 },
        });
        this.verdicts.push(verdict);
        if (!verdict.handled || verdict.prevented) {
            return false;
        }
        this.send({ type: "resizeupdate", id, edges: edge, delta });
        this.send({ type: "resizeend", id });
        return true;
    }

    doubleClickEdge(id, edges) {
        this.send({
            type: "edgedoubleclick",
            id,
            edges: { left: false, right: false, top: false, bottom: false, ...edges },
        });
    }

    /** Asks what size a new window on a workspace would get. */
    newWindowSize(workspace = this.activeWorkspace().id, info = {}) {
        return this.send({
            type: "newwindowsize",
            workspace,
            rules: info.rules ?? {},
            minSize: info.minSize ?? { w: 0, h: 0 },
            maxSize: info.maxSize ?? { w: 0, h: 0 },
            borders: info.borders ?? { w: 0, h: 0 },
        });
    }

    /** Sends a pointer event to a node, like a click on what the engine drew. */
    pointer(node, event = "click", init = {}) {
        if (event === "pointerdown") this.grab = null;
        this.send({
            type: "pointer",
            node,
            event,
            pointer: { x: 0, y: 0 },
            button: 1,
            modifiers: { shift: false, ctrl: false, alt: false, super: false },
            ...init,
        });
    }

    /**
     * The pointer move or resize that the engine began in the last press, as `{ kind: "move",
     * window }` or `{ kind: "resize", window, edges }`, or null. Taking it is what the input code
     * does to start the grab.
     */
    takeGrab() {
        const grab = this.grab;
        this.grab = null;
        return grab;
    }

    /**
     * Saves the engine's state and creates the engine again, like reloading its file. With
     * `{ state: false }` the engine starts without saved state, like a different engine took over.
     */
    reload({ state: keep = true } = {}) {
        const state = keep ? JSON.parse(prelude.saveAll()) : null;
        // What the old instance turned off around windows comes back on, since what it drew is gone.
        for (const win of this.windows.values()) win.decorations = { ...ALL_DECORATIONS };
        this.grab = null;
        this.told = { monitors: new Map(), workspaces: new Map(), windows: new Map() };
        this.toldGroups = null;
        this.toldOptions = null;
        this.timers.clear();
        // The engine that held the keyboard is gone, so nobody is told.
        this.keyboard.grabbed = false;
        this.queued = [];
        this.toldKeyboardAvailable = null;
        // The new instance has no search.
        this.search = null;
        this.toldSearch = null;
        for (const [id, tree] of this.trees) {
            tree.reset([...this.windows.values()].filter((win) => win.workspace === id).map((win) => win.id));
        }
        this.create(state);
    }

    /** The state that the engine exposes. */
    exposedState() {
        return JSON.parse(prelude.exposedState());
    }

    rect(id) {
        return this.windows.get(id).rect;
    }

    /** Rect of a window relative to the view of its workspace, which is what's on screen. */
    screenRect(id) {
        const win = this.windows.get(id);
        const camera = this.workspaces.get(win.workspace).camera;
        const rect = win.rect;
        return { x: rect.x - camera.x, y: rect.y - camera.y, w: rect.w, h: rect.h };
    }

    isVisible(id) {
        return this.windows.get(id).visible;
    }

    mode(id) {
        return this.windows.get(id).mode;
    }

    camera(workspace = this.activeWorkspace().id) {
        return this.workspaces.get(workspace).camera;
    }

    /** The windows of a workspace from the bottom to the top. */
    stacking(workspace = this.activeWorkspace().id) {
        return this.trees.get(workspace).windows();
    }

    /** The tree of a workspace. */
    tree(workspace = this.activeWorkspace().id) {
        return this.trees.get(workspace);
    }

    /**
     * Gives windows the size they were placed at, within their limits. A window with `stubborn`
     * set to a size keeps that size, like a terminal that rounds its size.
     */
    settleSizes() {
        for (const win of this.windows.values()) {
            if (!win.rect) continue;
            if (win.mode === "fullscreen") {
                win.size = { ...this.monitors.get(this.workspaces.get(win.workspace).monitor).size };
                continue;
            }
            if (win.stubborn) {
                win.size = { ...win.stubborn };
                continue;
            }
            const clamp = (v, min, max) => Math.max(min, max > 0 ? Math.min(v, max) : v);
            win.size = {
                w: clamp(win.rect.w, win.minSize.w, win.maxSize.w),
                h: clamp(win.rect.h, win.minSize.h, win.maxSize.h),
            };
        }
    }

    /** One line per window in the order they opened, for snapshot-style asserts. */
    format() {
        return [...this.windows.values()]
            .map((win) => {
                const r = win.rect;
                const flags = [
                    win.id === this.focused() ? "focused" : null,
                    win.visible ? null : "hidden",
                    win.mode === "normal" ? null : win.mode,
                ].filter(Boolean);
                return `${win.id}: ${r.w}×${r.h} at ${r.x},${r.y}${flags.length ? " " + flags.join(" ") : ""}`;
            })
            .join("\n");
    }
}

/** A workspace's tree of nodes, with the ops applied like bento does. */
export class Tree {
    constructor(workspace) {
        this.rootId = `r${workspace}`;
        this.nodes = new Map([[this.rootId, { kind: "root", props: {}, children: [], parent: null }]]);
    }

    has(id) {
        return this.nodes.has(id);
    }

    node(id) {
        return this.nodes.get(id);
    }

    add(id) {
        this.nodes.set(id, { kind: "window", props: {}, children: [], parent: null });
        this.append(this.rootId, id, null);
    }

    remove(id) {
        this.detach(id);
        this.drop(id);
    }

    create(op) {
        this.nodes.set(op.id, { kind: op.kind, props: { ...op.props }, children: [], parent: null });
    }

    append(parent, child, before) {
        this.detach(child);
        const parentNode = this.nodes.get(parent);
        const at = before === null || before === undefined ? parentNode.children.length : parentNode.children.indexOf(before);
        parentNode.children.splice(at, 0, child);
        this.nodes.get(child).parent = parent;
    }

    detach(id) {
        const node = this.nodes.get(id);
        if (node?.parent) {
            const parent = this.nodes.get(node.parent);
            parent.children.splice(parent.children.indexOf(id), 1);
            node.parent = null;
        }
    }

    drop(id) {
        const node = this.nodes.get(id);
        this.nodes.delete(id);
        node?.children.forEach((child) => this.drop(child));
    }

    take(id) {
        this.detach(id);
        const taken = new Map();
        const visit = (node) => {
            taken.set(node, this.nodes.get(node));
            this.nodes.delete(node);
            taken.get(node).children.forEach(visit);
        };
        visit(id);
        return { id, taken };
    }

    put(subtree, parent = this.rootId, before = null) {
        for (const [id, node] of subtree.taken) {
            this.nodes.set(id, node);
        }
        this.append(parent, subtree.id, before);
    }

    apply(op) {
        if (op.op === "set") {
            Object.assign(this.nodes.get(op.id).props, op.props);
        } else if (op.op === "remove") {
            this.detach(op.id);
            this.drop(op.id);
        }
    }

    reset(windows) {
        this.nodes = new Map([[this.rootId, { kind: "root", props: {}, children: [], parent: null }]]);
        windows.forEach((id) => this.add(`w${id}`));
    }

    /** Ids from the bottom to the top, depth first. */
    order(id = this.rootId) {
        return [id, ...this.nodes.get(id).children.flatMap((child) => this.order(child))];
    }

    /** Ids of the windows from the bottom to the top. */
    windows() {
        return this.order()
            .filter((id) => this.nodes.get(id).kind === "window")
            .map((id) => Number(id.slice(1)));
    }
}
