// The objects that engines see: monitors, workspaces, windows and their cameras.
//
// bento reports what it knows about them in `sync`, and the `Session` keeps them current. Engines
// change them with method calls, which take effect here right away, so that what an engine reads
// matches what it just did, and are recorded for bento to carry out. bento answers by reporting
// what really happened in the next `sync`.

import { BentoEvent, EventTarget, dispatch } from "./events.js";
import { GroupSet } from "./groups.js";
import {
    TreeLog,
    attachWindowNode,
    createNode,
    createRoot,
    createWindowNode,
    detachNode,
    isRect,
    windowsOf,
} from "./nodes.js";

const SIZING_MODES = ["normal", "fullscreen", "maximized"];

function copyRect(rect) {
    return { x: rect.x, y: rect.y, w: rect.w, h: rect.h };
}

function sameRect(a, b) {
    return a !== null && b !== null && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}

function freezeSize(size) {
    return Object.freeze({ w: size.w, h: size.h });
}

function freezeRect(rect) {
    return Object.freeze(copyRect(rect));
}

function copyJson(value) {
    return value === undefined ? null : JSON.parse(JSON.stringify(value));
}


/** What the engine's calls changed, for bento to carry out. */
class Recorder {
    constructor() {
        this.logs = [];
        /** Window ops and workspace ops, in the order the engine made them. */
        this.ops = [];
        /** Windows whose rect, visibility or mode changed, with whether to animate. */
        this.placed = new Map();
        this.data = new Set();
        /** Workspace id to the id of the window that has its focus. */
        this.focus = new Map();
        this.cameras = new Map();
        this.monitorFocus = undefined;
        this.stateChanged = false;
    }
}

function monitorKey(d) {
    return JSON.stringify([d.size, d.scale, d.workingArea, d.left, d.right, d.up, d.down]);
}

function windowKey(d) {
    return JSON.stringify([d.appId, d.title, d.size, d.minSize, d.maxSize, d.urgent, d.rules]);
}

/**
 * The tiling area of a workspace: the monitor's working area minus the struts from the config,
 * starting at a physical pixel.
 */
function tilingAreaOf(view, config) {
    const parent = view.workingArea;
    const struts = config.struts;
    const scale = view.scale;
    const x = parent.x + struts.left;
    const y = parent.y + struts.top;
    const w = Math.max(0, parent.w - struts.left - struts.right);
    const h = Math.max(0, parent.h - struts.top - struts.bottom);
    const px = Math.ceil(x * scale) / scale;
    const py = Math.ceil(y * scale) / scale;
    return Object.freeze({
        x: px,
        y: py,
        w: w - Math.min(w, px - x),
        h: h - Math.min(h, py - y),
    });
}

export class Camera {
    #s;
    #d;

    constructor(session, data) {
        this.#s = session;
        this.#d = data;
    }

    get x() {
        return this.#d.camera.x;
    }

    get y() {
        return this.#d.camera.y;
    }

    move(to, opts = {}) {
        const x = to.x ?? this.#d.camera.x;
        const y = to.y ?? this.#d.camera.y;
        if (!Number.isFinite(x) || !Number.isFinite(y)) {
            throw new Error(`invalid camera position: ${JSON.stringify(to)}`);
        }
        if (this.#s.querying) {
            return true;
        }
        this.#d.camera = { x, y };
        this.#s.rec.cameras.set(this.#d.id, {
            workspace: this.#d.id,
            x,
            y,
            animate: opts.animate ?? true,
            velocity: Number.isFinite(opts.velocity) ? opts.velocity : 0,
        });
        return true;
    }
}

export class Monitor extends EventTarget {
    #s;
    #d;

    constructor(session, data) {
        super();
        this.#s = session;
        this.#d = data;
    }

    get name() {
        return this.#d.name;
    }

    get size() {
        return this.#d.size;
    }

    get scale() {
        return this.#d.scale;
    }

    get workingArea() {
        return this.#d.workingArea;
    }

    get workspaces() {
        return Object.freeze(this.#s.monitorWorkspaces(this.#d.name).map((ws) => ws.facade));
    }

    get activeWorkspace() {
        return this.#s.workspaces.get(this.#d.active)?.facade ?? null;
    }

    get focused() {
        return this.#s.focusedMonitor === this.#d.name;
    }

    get left() {
        return this.#s.monitors.get(this.#d.left)?.facade ?? null;
    }

    get right() {
        return this.#s.monitors.get(this.#d.right)?.facade ?? null;
    }

    get up() {
        return this.#s.monitors.get(this.#d.up)?.facade ?? null;
    }

    get down() {
        return this.#s.monitors.get(this.#d.down)?.facade ?? null;
    }

    focus() {
        if (this.#s.querying) {
            return true;
        }
        this.#s.focusedMonitor = this.#d.name;
        this.#s.rec.monitorFocus = this.#d.name;
        return true;
    }

    round(value) {
        return Math.round(value * this.#d.scale) / this.#d.scale;
    }
}

export class Workspace extends EventTarget {
    #s;
    #d;
    #camera;

    constructor(session, data) {
        super();
        this.#s = session;
        this.#d = data;
        this.#camera = new Camera(session, data);
    }

    get id() {
        return this.#d.id;
    }

    get name() {
        return this.#d.name;
    }

    get monitor() {
        return this.#s.monitors.get(this.#d.monitor)?.facade ?? null;
    }

    get index() {
        return this.#d.index;
    }

    get windows() {
        return Object.freeze(windowsOf(this.#d.root));
    }

    get focused() {
        return this.#s.windows.get(this.#d.focused)?.facade ?? null;
    }

    get active() {
        return this.#s.isActive(this.#d);
    }

    get camera() {
        return this.#camera;
    }

    get config() {
        return this.#d.config;
    }

    get tilingArea() {
        return tilingAreaOf(this.#d.view, this.#d.config);
    }

    get tree() {
        return this.#d.root;
    }

    show(opts = {}) {
        if (this.#d.monitor === null) {
            return false;
        }
        if (this.#s.querying) {
            return true;
        }
        this.#s.monitors.get(this.#d.monitor).active = this.#d.id;
        this.#s.rec.ops.push({ op: "show", workspace: this.#d.id, animate: opts.animate ?? true });
        return true;
    }

    moveTo(monitor, index) {
        const target = this.#s.monitors.get(monitor?.name);
        if (target === undefined || target.facade !== monitor) {
            throw new Error("not a monitor of this session");
        }
        if (index !== undefined && !Number.isInteger(index)) {
            throw new Error(`invalid workspace index: ${index}`);
        }
        if (this.#s.querying) {
            return true;
        }
        this.#s.moveWorkspace(this.#d, target, index);
        this.#s.rec.ops.push({
            op: "moveWorkspace",
            workspace: this.#d.id,
            monitor: target.name,
            index: index ?? null,
        });
        return true;
    }

    rename(name) {
        if (name !== null && typeof name !== "string") {
            throw new TypeError("a workspace's name must be a string or null");
        }
        if (this.#s.querying) {
            return true;
        }
        this.#d.name = name;
        this.#s.rec.ops.push({ op: "rename", workspace: this.#d.id, name });
        return true;
    }
}

export class Window extends EventTarget {
    #s;
    #d;

    constructor(session, data) {
        super();
        this.#s = session;
        this.#d = data;
    }

    get id() {
        return this.#d.id;
    }

    get appId() {
        return this.#d.appId;
    }

    get title() {
        return this.#d.title;
    }

    get workspace() {
        return this.#s.workspaces.get(this.#d.workspace)?.facade ?? null;
    }

    get rect() {
        return this.#d.rect;
    }

    get size() {
        return this.#d.size;
    }

    get minSize() {
        return this.#d.minSize;
    }

    get maxSize() {
        return this.#d.maxSize;
    }

    get borders() {
        return this.#d.borders;
    }

    get mode() {
        return this.#d.mode;
    }

    get visible() {
        return this.#d.visible;
    }

    get urgent() {
        return this.#d.urgent;
    }

    get rules() {
        return this.#d.rules;
    }

    get group() {
        return this.#s.groups.of(this.#d.id);
    }

    get node() {
        return this.#d.node;
    }

    get data() {
        return this.#d.data;
    }

    /** Throws if the window is gone from the engine, or was never in it. */
    #check() {
        if (this.#s.windows.get(this.#d.id) !== this.#d) {
            throw new Error(`window ${this.#d.id} is not in the layout engine`);
        }
    }

    place(rect, opts = {}) {
        this.#check();
        if (!isRect(rect)) {
            throw new Error(`invalid rect for window ${this.#d.id}: ${JSON.stringify(rect)}`);
        }
        const d = this.#d;
        const fits = (v, min, max) => v >= min && (max === 0 || v <= max);
        const accepted = fits(rect.w, d.minSize.w, d.maxSize.w) && fits(rect.h, d.minSize.h, d.maxSize.h);
        if (this.#s.querying || sameRect(d.rect, rect)) {
            return accepted;
        }
        d.rect = freezeRect(rect);
        this.#s.markPlaced(d, opts.animate ?? true);
        d.node.set({ rect: d.rect });
        return accepted;
    }

    focus() {
        this.#check();
        if (this.#s.querying) {
            return true;
        }
        this.#s.focusWindow(this.#s.workspaces.get(this.#d.workspace), this.#d.id);
        return true;
    }

    raise() {
        this.#check();
        const node = this.#d.node;
        node.parent.append(node);
        return true;
    }

    moveTo(workspace) {
        this.#check();
        const target = this.#s.workspaces.get(workspace?.id);
        if (target === undefined || target.facade !== workspace) {
            throw new Error("not a workspace of this session");
        }
        if (this.#s.querying) {
            return true;
        }
        if (target.id !== this.#d.workspace) {
            this.#s.moveWindow(this.#d, target);
        }
        return true;
    }

    setMode(mode) {
        this.#check();
        if (!SIZING_MODES.includes(mode)) {
            throw new Error(`invalid sizing mode: ${mode}`);
        }
        if (this.#s.querying) {
            return true;
        }
        if (this.#d.mode !== mode) {
            this.#d.mode = mode;
            this.#s.markPlaced(this.#d, undefined);
        }
        return true;
    }

    setVisible(visible) {
        this.#check();
        if (this.#s.querying) {
            return true;
        }
        if (this.#d.visible !== !!visible) {
            this.#d.visible = !!visible;
            this.#s.markPlaced(this.#d, undefined);
        }
        return true;
    }

    setData(data) {
        this.#check();
        // Check that it survives the trip through JSON.
        const copy = copyJson(data);
        if (this.#s.querying) {
            return;
        }
        this.#d.data = copy;
        this.#s.rec.data.add(this.#d.id);
    }
}

/** What `create(bento)` receives. */
export class Bento extends EventTarget {
    #s;

    constructor(session) {
        super();
        this.#s = session;
    }

    get monitors() {
        return Object.freeze([...this.#s.monitors.values()].map((d) => d.facade));
    }

    get focusedMonitor() {
        return this.#s.monitors.get(this.#s.focusedMonitor)?.facade ?? null;
    }

    get focused() {
        return this.#s.focusedWindow()?.facade ?? null;
    }

    get windows() {
        return Object.freeze([...this.#s.windows.values()].map((d) => d.facade));
    }

    get groups() {
        return Object.freeze(this.#s.groups.facades);
    }

    get options() {
        return this.#s.options;
    }

    get savedState() {
        return this.#s.savedState;
    }

    window(id) {
        return this.#s.windows.get(id)?.facade ?? null;
    }

    workspace(id) {
        return this.#s.workspaces.get(id)?.facade ?? null;
    }

    monitor(name) {
        return this.#s.monitors.get(name)?.facade ?? null;
    }

    createGroup(windows, active) {
        return this.#s.groups.create(windows, active);
    }

    exposeState(provider) {
        if (typeof provider !== "function") {
            throw new TypeError("the state provider must be a function");
        }
        this.#s.stateProvider = provider;
    }

    stateChanged() {
        if (!this.#s.querying) {
            this.#s.rec.stateChanged = true;
        }
    }

    createNode(kind, props) {
        return createNode(this.#s.log, kind, props);
    }

    log(...args) {
        this.#s.rec.logs.push(
            args.map((arg) => (typeof arg === "string" ? arg : JSON.stringify(arg))).join(" "),
        );
    }
}

/** Everything the engine sees, and what it changed since bento last looked. */
export class Session {
    constructor(meta, given, savedState) {
        this.meta = meta;
        this.log = new TreeLog();
        this.rec = new Recorder();
        this.monitors = new Map();
        this.workspaces = new Map();
        this.windows = new Map();
        this.groups = new GroupSet(this);
        this.focusedMonitor = null;
        this.stateProvider = null;
        this.savedState = savedState ?? null;
        this.setOptions(given);
        this.bento = new Bento(this);
    }

    get querying() {
        return this.log.querying;
    }

    /** Sets the engine's options, and returns whether they changed. */
    setOptions(given) {
        // bento checks the options, fills in defaults and converts them by type before sending.
        const options = Object.freeze(copyJson(given ?? {}));
        const changed = JSON.stringify(options) !== JSON.stringify(this.options);
        this.options = options;
        return changed;
    }

    /** Runs a query, which changes nothing. */
    query(call) {
        const was = this.log.querying;
        this.log.querying = true;
        try {
            return call();
        } finally {
            this.log.querying = was;
        }
    }

    monitorWorkspaces(name) {
        return [...this.workspaces.values()]
            .filter((d) => d.monitor === name)
            .sort((a, b) => a.index - b.index);
    }

    isActive(ws) {
        return ws.monitor !== null && this.monitors.get(ws.monitor)?.active === ws.id;
    }

    focusedWindow() {
        const monitor = this.monitors.get(this.focusedMonitor);
        const ws = this.workspaces.get(monitor?.active);
        return this.windows.get(ws?.focused) ?? null;
    }

    focusKey() {
        const monitor = this.monitors.get(this.focusedMonitor);
        const ws = this.workspaces.get(monitor?.active);
        return JSON.stringify([this.focusedMonitor, ws?.id ?? null, ws?.focused ?? null]);
    }

    /**
     * Fires `focuschange` for focus the engine moved itself since the focus was `seen`, until
     * listeners stop moving it.
     */
    fireFocusChanges(seen) {
        // Bounded, so listeners that keep moving focus can't hang the call.
        for (let i = 0; i < 8 && this.focusKey() !== seen; i++) {
            seen = this.focusKey();
            dispatch(this.bento, new BentoEvent("focuschange", this.bento));
        }
    }

    /** Fires `group` events for what changed in groups since listeners were told, until they settle. */
    fireGroupChanges() {
        // Bounded, so listeners that keep changing groups can't hang the call.
        for (let i = 0; i < 8; i++) {
            const fire = this.groups.announce();
            if (fire.length === 0) {
                return;
            }
            for (const call of fire) {
                call();
            }
        }
    }

    markPlaced(win, animate) {
        if (animate !== undefined) {
            this.rec.placed.set(win.id, animate);
        } else if (!this.rec.placed.has(win.id)) {
            this.rec.placed.set(win.id, true);
        }
    }

    focusWindow(ws, id) {
        if (ws.focused !== id) {
            ws.focused = id;
            this.rec.focus.set(ws.id, id);
        }
    }

    /** Makes the workspace have no focused window. */
    clearFocus(ws) {
        this.focusWindow(ws, null);
    }

    /** Puts a window on another workspace, with its node last in that workspace's tree. */
    relocateWindow(win, to) {
        const from = this.workspaces.get(win.workspace);
        if (from !== undefined && from.focused === win.id) {
            from.focused = null;
        }
        win.workspace = to.id;
        // It hasn't been placed on the new workspace's canvas, so placing it anywhere counts.
        win.rect = null;
        attachWindowNode(to.root, win.node);
    }

    /** Moves a window to another workspace, for the engine's `moveTo`. */
    moveWindow(win, to) {
        this.relocateWindow(win, to);
        this.log.record({ op: "append", parent: to.root.id, child: win.node.id, before: null });
        this.rec.ops.push({ op: "moveWindow", window: win.id, workspace: to.id });
    }

    /** Moves a workspace to a monitor, at `index` in its stack or last. */
    moveWorkspace(ws, monitor, index) {
        const from = ws.monitor;
        const stack = this.monitorWorkspaces(monitor.name).filter((other) => other !== ws);
        stack.splice(index === undefined || index === null ? stack.length : Math.max(0, index), 0, ws);
        ws.monitor = monitor.name;
        stack.forEach((other, idx) => {
            other.index = idx;
        });
        if (from !== null && from !== monitor.name) {
            this.monitorWorkspaces(from).forEach((other, idx) => {
                other.index = idx;
            });
        }
    }

    updateWindow(d, info) {
        d.appId = info.appId;
        d.title = info.title;
        d.size = freezeSize(info.size);
        d.minSize = freezeSize(info.minSize);
        d.maxSize = freezeSize(info.maxSize);
        d.borders = freezeSize(info.borders);
        d.mode = info.mode;
        d.urgent = info.urgent;
        d.rules = Object.freeze({ ...info.rules });
    }

    /**
     * Brings what engines see up to date with what bento reports, and returns what to tell them
     * about it, as calls to make once everything is current.
     */
    applySync(sync) {
        const before = new Map();
        for (const d of this.monitors.values()) {
            before.set(d, monitorKey(d));
        }
        for (const d of this.workspaces.values()) {
            before.set(d, this.workspaceKey(d));
        }
        for (const info of sync.windows ?? []) {
            const d = this.windows.get(info.id);
            if (d !== undefined) {
                before.set(d, windowKey(d));
            }
        }
        const focusBefore = this.focusKey();
        const configBefore = new Map([...this.workspaces.values()].map((d) => [d, JSON.stringify(d.config)]));

        const addedMonitors = [];
        for (const info of sync.monitors ?? []) {
            let d = this.monitors.get(info.name);
            if (d === undefined) {
                d = { name: info.name };
                d.facade = new Monitor(this, d);
                this.monitors.set(info.name, d);
                addedMonitors.push(d);
            }
            d.size = freezeSize(info.size);
            d.scale = info.scale;
            d.workingArea = freezeRect(info.workingArea);
            d.active = info.active;
            d.left = info.left ?? null;
            d.right = info.right ?? null;
            d.up = info.up ?? null;
            d.down = info.down ?? null;
        }

        const addedWorkspaces = [];
        for (const info of sync.workspaces ?? []) {
            let d = this.workspaces.get(info.id);
            if (d === undefined) {
                d = { id: info.id };
                d.facade = new Workspace(this, d);
                d.root = createRoot(this.log, info.id);
                this.workspaces.set(info.id, d);
                addedWorkspaces.push(d);
            }
            d.name = info.name;
            d.monitor = info.monitor;
            d.index = info.index;
            d.view = {
                size: freezeSize(info.view.size),
                workingArea: freezeRect(info.view.workingArea),
                scale: info.view.scale,
            };
            d.camera = { x: info.camera.x, y: info.camera.y };
            d.config = Object.freeze({
                gaps: info.config.gaps,
                struts: Object.freeze({ ...info.config.struts }),
            });
            d.focused = info.focused;
        }

        for (const info of sync.windows ?? []) {
            const d = this.windows.get(info.id);
            if (d === undefined) {
                continue;
            }
            this.updateWindow(d, info);
            const ws = this.workspaces.get(info.workspace);
            if (ws !== undefined && d.workspace !== info.workspace) {
                this.relocateWindow(d, ws);
            }
        }

        if (sync.groups !== undefined) {
            this.groups.apply(sync.groups);
        }

        if ("focusedMonitor" in sync) {
            this.focusedMonitor = sync.focusedMonitor;
        }
        const optionsChanged = "options" in sync && this.setOptions(sync.options);

        const removedMonitors = (sync.removedMonitors ?? [])
            .map((name) => this.monitors.get(name))
            .filter((d) => d !== undefined);
        const removedWorkspaces = (sync.removedWorkspaces ?? [])
            .map((id) => this.workspaces.get(id))
            .filter((d) => d !== undefined);

        const fire = [];
        const event = (target, type, init = {}) => () => dispatch(target, new BentoEvent(type, target, init));
        for (const d of addedMonitors) {
            fire.push(event(this.bento, "monitoradd", { monitor: d.facade }));
        }
        for (const d of addedWorkspaces) {
            fire.push(event(this.bento, "workspaceadd", { workspace: d.facade }));
        }
        for (const d of this.monitors.values()) {
            if (before.has(d) && before.get(d) !== monitorKey(d)) {
                fire.push(event(d.facade, "change"));
            }
        }
        for (const d of this.workspaces.values()) {
            if (before.has(d) && before.get(d) !== this.workspaceKey(d)) {
                fire.push(event(d.facade, "change"));
            }
        }
        for (const info of sync.windows ?? []) {
            const d = this.windows.get(info.id);
            if (d !== undefined && before.has(d) && before.get(d) !== windowKey(d)) {
                fire.push(event(d.facade, "change"));
            }
        }
        if (sync.groups !== undefined) {
            fire.push(...this.groups.announce());
        }
        if (focusBefore !== this.focusKey()) {
            fire.push(event(this.bento, "focuschange"));
        }
        const configChanged = [...this.workspaces.values()].some(
            (d) => configBefore.has(d) && configBefore.get(d) !== JSON.stringify(d.config),
        );
        if (optionsChanged || configChanged) {
            fire.push(event(this.bento, "configchange"));
        }
        for (const d of removedMonitors) {
            fire.push(event(d.facade, "remove"));
            fire.push(() => {
                this.monitors.delete(d.name);
                if (this.focusedMonitor === d.name) {
                    this.focusedMonitor = null;
                }
            });
        }
        for (const d of removedWorkspaces) {
            fire.push(event(d.facade, "remove"));
            fire.push(() => {
                this.workspaces.delete(d.id);
                detachNode(d.root);
            });
        }
        return fire;
    }

    workspaceKey(d) {
        return JSON.stringify([d.name, d.monitor, d.index, this.isActive(d)]);
    }

    /** Adds a window that bento reports, with its node last in its workspace's tree. */
    addWindow(info) {
        if (this.windows.has(info.id)) {
            throw new Error(`window ${info.id} is already in the layout engine`);
        }
        const ws = this.workspaces.get(info.workspace);
        if (ws === undefined) {
            throw new Error(`window ${info.id} is on workspace ${info.workspace}, which doesn't exist`);
        }
        const d = { id: info.id, workspace: ws.id, rect: null, visible: true, data: info.data ?? null };
        this.updateWindow(d, info);
        d.facade = new Window(this, d);
        d.node = createWindowNode(this.log, d.facade);
        this.windows.set(d.id, d);
        attachWindowNode(ws.root, d.node);
        return d;
    }

    /** A window that isn't in the engine, for events about a window that left it, like a drag. */
    detachedWindow(info) {
        const d = { id: info.id, workspace: info.workspace, rect: null, visible: true, data: info.data ?? null, node: null };
        this.updateWindow(d, info);
        d.facade = new Window(this, d);
        return d.facade;
    }

    /** Takes a window away after the engine has been told, and clears what refers to it. */
    deleteWindow(d) {
        this.windows.delete(d.id);
        const ws = this.workspaces.get(d.workspace);
        if (ws !== undefined && ws.focused === d.id) {
            ws.focused = null;
        }
        detachNode(d.node);
        this.rec.placed.delete(d.id);
        this.rec.data.delete(d.id);
    }

    /** What the engine did since the last call, as bento reads it. */
    takeOutput() {
        const rec = this.rec;
        this.rec = new Recorder();
        const out = { logs: rec.logs };
        if (rec.ops.length > 0) {
            out.ops = rec.ops;
        }
        const windows = [];
        for (const [id, animate] of rec.placed) {
            const d = this.windows.get(id);
            if (d !== undefined) {
                windows.push({ id, rect: d.rect, visible: d.visible, mode: d.mode, animate });
            }
        }
        if (windows.length > 0) {
            out.windows = windows;
        }
        const data = [...rec.data].filter((id) => this.windows.has(id));
        if (data.length > 0) {
            out.data = data.map((id) => [id, this.windows.get(id).data]);
        }
        if (rec.focus.size > 0) {
            out.focus = [...rec.focus];
        }
        if (rec.cameras.size > 0) {
            out.cameras = [...rec.cameras.values()];
        }
        if (rec.monitorFocus !== undefined) {
            out.monitorFocus = rec.monitorFocus;
        }
        const tree = this.log.take();
        if (tree.length > 0) {
            out.tree = tree;
        }
        if (rec.stateChanged) {
            out.stateChanged = true;
        }
        return out;
    }

    /** Throws away what a query recorded, keeping its log. */
    discardOutput() {
        const logs = this.rec.logs;
        this.rec = new Recorder();
        this.rec.logs = logs;
        this.log.take();
    }
}
