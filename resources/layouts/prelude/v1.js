// Runs API 1 engines (`bento-layout-v1.d.ts`) on top of the API 2 objects and events.
//
// An API 1 engine runs one instance per workspace, and sees that workspace through a `Workspace`
// facade. The adapter creates an instance for each workspace, turns events into calls on the
// instance's `Layout`, and turns the facade's calls into calls on the API 2 objects.
//
// Some of API 1 doesn't exist any more:
// - bento's config has nothing for columns, tab indicators or hints, so `LayoutConfig` holds niri's
//   defaults for them, and window rules reach the engine as their defaults;
// - bento draws no tab bars or hints, so `setTabBar` and `setHint` draw nodes, and clicking a tab
//   calls `Layout.activate`;
// - `setActiveInColumn` and `ipcPosition` have no effect.

import { isRect } from "./nodes.js";

const DECELERATION_TOUCHPAD = 0.997;

const V1_CONFIG = Object.freeze({
    centerFocusedColumn: "never",
    alwaysCenterSingleColumn: false,
    defaultColumnWidth: Object.freeze({ proportion: 0.5 }),
    presetColumnWidths: Object.freeze([{ proportion: 1 / 3 }, { proportion: 0.5 }, { proportion: 2 / 3 }]),
    presetWindowHeights: Object.freeze([{ proportion: 1 / 3 }, { proportion: 0.5 }, { proportion: 2 / 3 }]),
    defaultColumnDisplay: "normal",
    tabIndicator: Object.freeze({
        off: false,
        position: "left",
        width: 4,
        gap: 5,
        placeWithinColumn: false,
        hideWhenSingleTab: false,
    }),
});

const V1_RULES = Object.freeze({
    defaultWidth: null,
    defaultHeight: null,
    defaultColumnDisplay: "normal",
    openFullWidth: false,
});

const HINT_PROPS = {
    background: "#7fc8ff80",
    border: { width: 2, color: "#7fc8ff" },
    cornerRadius: 8,
    transitions: { rect: "window-movement" },
};
const TAB_ACTIVE = "#7fc8ff";
const TAB_INACTIVE = "#505050";

const wrapped = new WeakMap();

/** A window as API 1 engines see it, on top of the API 2 window. */
class V1Window {
    constructor(window) {
        wrapped.set(this, window);
    }

    get id() {
        return wrapped.get(this).id;
    }

    get appId() {
        return wrapped.get(this).appId;
    }

    get title() {
        return wrapped.get(this).title;
    }

    get size() {
        return wrapped.get(this).size;
    }

    get minSize() {
        return wrapped.get(this).minSize;
    }

    get maxSize() {
        return wrapped.get(this).maxSize;
    }

    get borders() {
        return wrapped.get(this).borders;
    }

    get mode() {
        return wrapped.get(this).mode;
    }

    get rules() {
        return V1_RULES;
    }

    get isUrgent() {
        return wrapped.get(this).urgent;
    }

    get rect() {
        return wrapped.get(this).rect;
    }

    get visible() {
        return wrapped.get(this).visible;
    }

    get activeInColumn() {
        return null;
    }

    get data() {
        return wrapped.get(this).data;
    }
}

function copyRect(rect) {
    return { x: rect.x, y: rect.y, w: rect.w, h: rect.h };
}

/** A tab bar, as bento's tab indicator drew it: one segment per tab along the area's left. */
function tabKey(bar) {
    return [bar.area.x, bar.area.y, bar.area.w, bar.area.h, bar.active, ...bar.windows].join();
}

/** An API 1 engine for one workspace, with the `Workspace` it sees. */
class V1Instance {
    constructor(adapter, ws) {
        this.adapter = adapter;
        this.ws = ws;
        this.byId = new Map();
        /** Nodes that show the engine's hint and the drop target. */
        this.overlays = { hint: null, drop: null };
        this.tabBars = new Map();
        this.swiping = false;
        this.velocity = 0;
        this.facade = this.makeWorkspace();
        this.layout = adapter.module.default(this.facade);
        if (this.layout === null || typeof this.layout !== "object") {
            throw new Error("the layout engine's default export must return an object");
        }
        for (const required of ["windowAdded", "windowRemoved"]) {
            if (typeof this.layout[required] !== "function") {
                throw new Error(`the layout engine must have a ${required} method`);
            }
        }
    }

    get bento() {
        return this.adapter.session.bento;
    }

    /** Wraps a window for this instance's engine. */
    add(window) {
        const win = new V1Window(window);
        this.byId.set(window.id, win);
        return win;
    }

    /** Forgets a window, and removes it from tab bars. */
    drop(window) {
        const win = this.byId.get(window.id);
        this.byId.delete(window.id);
        for (const [key, bar] of this.tabBars) {
            if (bar.windows.includes(window.id)) {
                const windows = bar.windows.filter((other) => other !== window.id);
                if (windows.length === 0) {
                    this.removeTabBar(key);
                } else {
                    bar.windows = windows;
                    if (bar.active === window.id) {
                        bar.active = windows[0];
                    }
                    this.drawTabBar(key);
                }
            }
        }
        return win;
    }

    own(win) {
        const found = win && this.byId.get(wrapped.get(win)?.id);
        if (found !== win) {
            throw new Error("not a window in this layout engine");
        }
        return wrapped.get(win);
    }

    /** Keeps what the engine draws above the windows that came after it. */
    raiseOverlays() {
        for (const node of Object.values(this.overlays)) {
            if (node !== null && node.parent !== null) {
                node.parent.append(node);
            }
        }
        for (const bar of this.tabBars.values()) {
            for (const node of bar.nodes) {
                node.parent?.append(node);
            }
        }
    }

    showBox(slot, rect) {
        const node = this.overlays[slot];
        if (rect === null) {
            node?.remove();
            return;
        }
        const box = node ?? this.bento.createNode("box", HINT_PROPS);
        this.overlays[slot] = box;
        box.set({ rect: copyRect(rect) });
        if (box.parent === null) {
            this.ws.tree.append(box);
        }
    }

    removeTabBar(key) {
        const bar = this.tabBars.get(key);
        if (bar !== undefined) {
            bar.nodes.forEach((node) => node.remove());
            this.tabBars.delete(key);
        }
    }

    drawTabBar(key) {
        const bar = this.tabBars.get(key);
        const count = bar.windows.length;
        while (bar.nodes.length > count) {
            bar.nodes.pop().remove();
        }
        while (bar.nodes.length < count) {
            const index = bar.nodes.length;
            const node = this.bento.createNode("box", { interactive: true, cornerRadius: 2 });
            node.addEventListener("click", () => this.activateTab(key, index));
            bar.nodes.push(node);
            this.ws.tree.append(node);
        }
        const { x, y, h } = bar.area;
        const { width, gap } = V1_CONFIG.tabIndicator;
        const step = h / count;
        bar.nodes.forEach((node, i) => {
            node.set({
                rect: { x: x - gap - width, y: y + i * step, w: width, h: Math.max(0, step - 2) },
                background: bar.windows[i] === bar.active ? TAB_ACTIVE : TAB_INACTIVE,
            });
        });
    }

    activateTab(key, index) {
        const win = this.byId.get(this.tabBars.get(key)?.windows[index]);
        if (win === undefined) {
            return;
        }
        if (typeof this.layout.activate === "function") {
            this.layout.activate(win);
        } else {
            wrapped.get(win).focus();
        }
    }

    makeWorkspace() {
        const self = this;
        const ws = this.ws;
        const round = (value) => ws.monitor?.round(value) ?? Math.round(value);
        const session = () => self.adapter.session;

        return Object.freeze({
            get windows() {
                return ws.windows.filter((w) => self.byId.has(w.id)).map((w) => self.byId.get(w.id));
            },
            get focused() {
                return ws.focused === null ? null : (self.byId.get(ws.focused.id) ?? null);
            },
            get view() {
                const monitor = ws.monitor;
                const size = monitor?.size ?? { w: 1280, h: 720 };
                return Object.freeze({
                    size,
                    workingArea: monitor?.workingArea ?? { x: 0, y: 0, w: size.w, h: size.h },
                    tilingArea: ws.tilingArea,
                    scale: monitor?.scale ?? 1,
                });
            },
            get camera() {
                return { x: ws.camera.x, y: ws.camera.y };
            },
            get config() {
                return Object.freeze({ ...V1_CONFIG, gaps: ws.config.gaps, struts: ws.config.struts });
            },
            get options() {
                return self.bento.options;
            },
            window(id) {
                return self.byId.get(id) ?? null;
            },
            place(win, rect, opts = {}) {
                self.own(win).place(rect, opts);
            },
            setWindowData(win, data) {
                self.own(win).setData(data);
            },
            setVisible(win, visible) {
                self.own(win).setVisible(visible);
            },
            setActiveInColumn(win) {
                self.own(win);
            },
            setMode(win, mode) {
                self.own(win).setMode(mode);
            },
            raise(win) {
                self.own(win).raise();
            },
            focus(win) {
                if (win === null) {
                    session().clearFocus(session().workspaces.get(ws.id));
                } else {
                    self.own(win).focus();
                }
            },
            moveCamera(to, opts = {}) {
                if (!self.swiping) {
                    ws.camera.move(to, { ...opts, velocity: self.velocity });
                }
            },
            setHint(rect) {
                if (rect !== null && !isRect(rect)) {
                    throw new Error(`invalid hint rect: ${JSON.stringify(rect)}`);
                }
                self.showBox("hint", rect);
            },
            setTabBar(key, bar) {
                key = String(key);
                if (bar === null || bar === undefined) {
                    self.removeTabBar(key);
                    return;
                }
                if (!isRect(bar.area)) {
                    throw new Error(`invalid tab bar area: ${JSON.stringify(bar.area)}`);
                }
                const windows = bar.windows.map((win) => self.own(win).id);
                const active = self.own(bar.active).id;
                if (!windows.includes(active)) {
                    throw new Error("the active tab must be one of the tabs");
                }
                const next = { area: copyRect(bar.area), windows, active };
                const shown = self.tabBars.get(key);
                if (shown !== undefined && tabKey(shown) === tabKey(next)) {
                    return;
                }
                self.tabBars.set(key, { ...next, nodes: shown?.nodes ?? [] });
                self.drawTabBar(key);
            },
            get tabBars() {
                return [...self.tabBars.keys()];
            },
            tabIndicatorSpace(count) {
                const config = V1_CONFIG.tabIndicator;
                if (config.off || !config.placeWithinColumn || (config.hideWhenSingleTab && count === 1)) {
                    return { x: 0, y: 0, w: 0, h: 0 };
                }
                const size = Math.max(0, round(config.width) + round(config.gap));
                switch (config.position) {
                    case "left":
                        return { x: size, y: 0, w: size, h: 0 };
                    case "right":
                        return { x: 0, y: 0, w: size, h: 0 };
                    case "top":
                        return { x: 0, y: size, w: 0, h: size };
                    default:
                        return { x: 0, y: 0, w: 0, h: size };
                }
            },
            round,
            log(...args) {
                self.bento.log(...args);
            },
        });
    }
}

export class V1Adapter {
    constructor(session, module) {
        this.session = session;
        this.module = module;
        this.instances = new Map();
        this.watched = new WeakSet();
    }

    /** Creates an instance for each workspace, and starts following the session. */
    start() {
        const bento = this.session.bento;
        for (const d of this.session.workspaces.values()) {
            this.addInstance(d.facade);
        }
        for (const monitor of bento.monitors) {
            this.watchMonitor(monitor);
        }
        bento.addEventListener("monitoradd", (e) => this.watchMonitor(e.monitor));
        bento.addEventListener("workspaceadd", (e) => this.addInstance(e.workspace));
        bento.addEventListener("windowadd", (e) => this.onWindowAdd(e));
        bento.addEventListener("configchange", () => this.each((inst) => inst.layout.configChanged?.()));
        bento.addEventListener("action", (e) => this.onAction(e));
        bento.addEventListener("newwindowsize", (e) => this.onNewWindowSize(e));
        bento.addEventListener("unload", (e) => this.onUnload(e));
    }

    each(call) {
        for (const inst of this.instances.values()) {
            call(inst);
            inst.raiseOverlays();
        }
    }

    /** Hands windows that were already there to their workspaces' engines. */
    adopt(windows) {
        const states = this.session.savedState ?? {};
        for (const inst of this.instances.values()) {
            const mine = windows.filter((w) => w.workspace.id === inst.ws.id);
            const wins = mine.map((w) => {
                this.watch(w);
                return inst.add(w);
            });
            const focused = inst.ws.focused === null ? null : (inst.byId.get(inst.ws.focused.id) ?? null);
            if (typeof inst.layout.adopt === "function") {
                inst.layout.adopt(wins, focused, states[inst.ws.id] ?? null);
            } else {
                for (const win of wins) {
                    inst.layout.windowAdded(win, {
                        reason: "adopt",
                        activate: win === focused,
                        rect: win.rect,
                        pointer: null,
                        nextTo: null,
                    });
                }
            }
            inst.raiseOverlays();
        }
    }

    addInstance(ws) {
        let inst;
        try {
            inst = new V1Instance(this, ws);
        } catch (e) {
            this.session.bento.log(`error creating the layout engine for workspace ${ws.id}: ${e.message}`);
            return;
        }
        this.instances.set(ws.id, inst);

        ws.addEventListener("remove", () => this.instances.delete(ws.id));
        ws.addEventListener("swipebegin", () => {
            inst.swiping = true;
            inst.layout.gestureBegin?.();
        });
        ws.addEventListener("swipeupdate", (e) => {
            ws.camera.move({ x: ws.camera.x + e.delta }, { animate: false });
        });
        ws.addEventListener("swipeend", (e) => {
            inst.swiping = false;
            if (typeof inst.layout.gestureEnd === "function") {
                const projected = ws.camera.x - e.velocity / (1000 * Math.log(DECELERATION_TOUCHPAD));
                inst.velocity = e.velocity;
                try {
                    inst.layout.gestureEnd({ x: e.velocity, y: 0 }, { x: projected, y: ws.camera.y });
                } finally {
                    inst.velocity = 0;
                }
            }
        });
        const dropTarget = (e) => {
            if (typeof inst.layout.dropTarget !== "function") {
                return;
            }
            const rect = inst.layout.dropTarget(new V1Window(e.window), e.pointer, e.rect);
            if (rect !== null && rect !== undefined && !isRect(rect)) {
                throw new Error(`invalid drop target: ${JSON.stringify(rect)}`);
            }
            inst.showBox("drop", rect ?? null);
        };
        ws.addEventListener("dragenter", dropTarget);
        ws.addEventListener("dragmove", dropTarget);
        ws.addEventListener("dragleave", () => inst.showBox("drop", null));
    }

    watchMonitor(monitor) {
        monitor.addEventListener("change", () => {
            this.each((inst) => {
                if (inst.ws.monitor === monitor) {
                    inst.layout.viewChanged?.();
                }
            });
        });
    }

    instanceOf(window) {
        return this.instances.get(window.workspace.id);
    }

    /** Listens to a window, once, and passes what it hears to the instance it's on. */
    watch(window) {
        if (this.watched.has(window)) {
            return;
        }
        this.watched.add(window);
        const mine = () => {
            const inst = this.instanceOf(window);
            return inst === undefined ? null : { inst, win: inst.byId.get(window.id) };
        };

        window.addEventListener("remove", (e) => {
            const found = mine();
            if (found !== null && found.win !== undefined) {
                found.inst.drop(window);
                found.inst.layout.windowRemoved(found.win, { reason: e.reason });
                found.inst.raiseOverlays();
            }
        });
        window.addEventListener("move", (e) => {
            const from = this.instances.get(e.from.id);
            const to = this.instances.get(e.to.id);
            if (from !== undefined && from.byId.has(window.id)) {
                const win = from.drop(window);
                from.layout.windowRemoved(win, { reason: "move" });
                from.raiseOverlays();
            }
            if (to !== undefined) {
                const win = to.add(window);
                to.layout.windowAdded(win, {
                    reason: "move",
                    activate: e.activate,
                    rect: null,
                    pointer: null,
                    nextTo: null,
                });
                to.raiseOverlays();
            }
        });
        window.addEventListener("change", () => {
            const found = mine();
            found?.inst.layout.windowChanged?.(found.win);
        });
        window.addEventListener("moderequest", (e) => {
            const found = mine();
            if (found !== null && typeof found.inst.layout.modeRequested === "function") {
                e.preventDefault();
                found.inst.layout.modeRequested(found.win, e.mode, { ...e.request });
                found.inst.raiseOverlays();
            }
        });
        window.addEventListener("activaterequest", () => {
            const found = mine();
            // bento focuses the window after this, whatever the engine does.
            found?.inst.layout.activate?.(found.win);
        });
        window.addEventListener("resizebegin", (e) => {
            const found = mine();
            if (found === null || typeof found.inst.layout.resizeBegin !== "function") {
                // Without a listener, bento refuses the resize.
                e.preventDefault();
            } else if (found.inst.layout.resizeBegin(found.win, { edges: e.edges, delta: e.delta }) === false) {
                e.preventDefault();
            }
        });
        window.addEventListener("resizeupdate", (e) => {
            const found = mine();
            found?.inst.layout.resizeUpdate?.(found.win, { edges: e.edges, delta: e.delta });
        });
        window.addEventListener("resizeend", () => {
            const found = mine();
            found?.inst.layout.resizeEnd?.(found.win);
        });
    }

    onWindowAdd(e) {
        const window = e.window;
        const inst = this.instanceOf(window);
        if (inst === undefined) {
            return;
        }
        this.watch(window);
        const win = inst.add(window);
        inst.layout.windowAdded(win, {
            reason: e.reason,
            activate: e.activate,
            rect: e.rect,
            pointer: e.pointer,
            nextTo: e.nextTo === null ? null : (inst.byId.get(e.nextTo.id) ?? null),
        });
        inst.raiseOverlays();
    }

    onAction(e) {
        const ws = e.window === null ? this.session.bento.focusedMonitor?.activeWorkspace : e.window.workspace;
        const inst = this.instances.get(ws?.id);
        if (inst === undefined || typeof inst.layout.action !== "function") {
            return;
        }
        const target = e.window === null ? null : (inst.byId.get(e.window.id) ?? null);
        inst.layout.action(e.name, [...e.args], target);
        inst.raiseOverlays();
    }

    onNewWindowSize(e) {
        const inst = this.instances.get(e.workspace.id);
        if (inst === undefined || typeof inst.layout.newWindowSize !== "function") {
            return;
        }
        e.respondWith(
            inst.layout.newWindowSize({
                rules: V1_RULES,
                minSize: e.minSize,
                maxSize: e.maxSize,
                borders: e.borders,
            }),
        );
    }

    /** Saves each instance's state by workspace. */
    onUnload(e) {
        const states = {};
        for (const [id, inst] of this.instances) {
            if (typeof inst.layout.save !== "function") {
                continue;
            }
            try {
                states[id] = inst.layout.save() ?? null;
            } catch (error) {
                // The engine starts from the window positions instead.
            }
        }
        e.saveState(states);
    }
}
