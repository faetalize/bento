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

const DEFAULT_MONITOR = { size: { w: 1280, h: 720 }, scale: 1, workspaces: 2 };
const MAX_ROUNDS = 8;

export class TestHost {
    /**
     * Loads a layout engine's module and creates it for an empty session.
     *
     * `opts.monitors` lists the monitors, each `{ name, size, scale, workingArea, workspaces,
     * position }`; by default there's one, `"DP-1"`, 1280 × 720 with two workspaces. `opts.config`
     * overrides parts of the workspaces' config, `opts.options` are the engine's options and
     * `opts.state` the state the previous instance of the engine saved.
     */
    static async load(path, opts = {}) {
        const url = new URL(path, import.meta.url);
        const module = await import(url.href);
        return new TestHost(module, opts);
    }

    constructor(module, opts = {}) {
        this.meta = JSON.parse(prelude.load(module));
        this.options = opts.options ?? {};
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
        this.trees = new Map();
        this.logs = [];
        this.errors = [];
        this.exposed = null;
        /** Results of cancelable events, in order. */
        this.verdicts = [];

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
            camera: { x: 0, y: 0 },
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
            borders: win.borders,
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
        sync.focusedMonitor = this.focusedMonitor;
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
            sync.options === undefined &&
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
            { sync, event: { type: "create", options: this.engineOptions(), savedState: state, windows, focused: this.focused() } },
            true,
        );
        this.flush();
    }

    /** Sends an event, and carries out what the engine did. Returns the call's result. */
    send(event) {
        this.flush();
        const sync = this.sync();
        this.toldFocusedMonitor = this.focusedMonitor;
        const out = this.call({ sync, event });
        this.flush();
        return out.result;
    }

    /** Tells the engine about what changed because of what it did, until it settles. */
    flush() {
        for (let round = 0; round < MAX_ROUNDS; round++) {
            this.settleSizes();
            const sync = this.sync();
            if (this.isEmptySync(sync)) {
                return;
            }
            this.toldFocusedMonitor = this.focusedMonitor;
            this.call({ sync, event: { type: "sync" } });
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
                default:
                    throw new Error(`unknown op: ${op.op}`);
            }
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
            this.workspaces.get(camera.workspace).camera = { x: camera.x, y: camera.y };
            this.lastCamera = camera;
        }
        if (out.monitorFocus !== undefined) {
            this.focusedMonitor = out.monitorFocus;
        }
        if (out.stateChanged) {
            this.stateChanges = (this.stateChanges ?? 0) + 1;
        }
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

    /** bento's `ungroup-window`: the focused window leaves its group. */
    ungroupWindow() {
        const focused = this.focused();
        if (focused !== null) this.leaveGroup(focused);
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
     * can set `minSize`, `maxSize`, `borders`, `appId`, `title`, `rules`, `activate` (default true)
     * and `nextTo`.
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
            mode: "normal",
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
        const win = this.windows.get(id);
        this.leaveGroup(id);
        this.windows.delete(id);
        this.told.windows.delete(id);
        this.trees.get(win.workspace).remove(`w${id}`);
        const ws = this.workspaces.get(win.workspace);
        if (ws.focused === id) ws.focused = null;
        this.send({ type: "windowremove", id, reason });
    }

    /** Moves a window to another workspace like one of bento's own actions does. */
    move(id, workspace, activate = false) {
        const from = this.windows.get(id).workspace;
        this.moveWindow(id, workspace);
        this.told.windows.set(id, this.windowInfo(this.windows.get(id)));
        this.send({ type: "windowmove", id, from, to: workspace, activate });
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

    /**
     * The engine's options as bento sends them: `this.options`, written as in the .kdl file, with
     * defaults filled in and sizes turned into presets.
     */
    engineOptions() {
        const preset = (value) => {
            if (typeof value !== "string") return value;
            return value.endsWith("%")
                ? { proportion: Number(value.slice(0, -1)) / 100 }
                : { fixed: Number(value) };
        };
        const options = {};
        for (const spec of this.meta.options) {
            let value = spec.name in this.options ? this.options[spec.name] : spec.default;
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
        const info = this.windowInfo(win);
        const from = win.workspace;
        const rect = (pointer) => ({ x: pointer.x - 10, y: pointer.y - 10, w: 20, h: 20 });
        this.close(id, "drag");
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

    /** Saves the engine's state and creates the engine again, like reloading its file. */
    reload() {
        const state = JSON.parse(prelude.saveAll());
        this.told = { monitors: new Map(), workspaces: new Map(), windows: new Map() };
        this.toldGroups = null;
        this.toldOptions = null;
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

    /** Gives windows the size they were placed at, within their limits. */
    settleSizes() {
        for (const win of this.windows.values()) {
            if (!win.rect) continue;
            if (win.mode === "fullscreen") {
                win.size = { ...this.monitors.get(this.workspaces.get(win.workspace).monitor).size };
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
