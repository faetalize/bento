// Host side of bento's layout API, running next to the layout in the same JS engine.
//
// bento talks to layouts with JSON events and gets JSON results back. This module turns events into
// calls on layout objects, keeps the windows, camera and focus that layouts see in `Workspace`, and
// collects what changed into the result. The node test harness in `test/host.js` uses it the same
// way, so layouts behave the same in tests and in bento.
//
// See `bento-layout.d.ts` for the API that layouts see.

const API_VERSION = 1;

let layoutModule = null;
const instances = new Map();

/** Sets the layout module, with its `meta` and default export. Returns the layout's metadata. */
export function load(module) {
    const meta = module.meta;
    if (meta === null || typeof meta !== "object") {
        throw new Error("the layout must export `meta`");
    }
    if (meta.api !== API_VERSION) {
        throw new Error(`the layout is for API version ${meta.api}, bento has version ${API_VERSION}`);
    }
    if (typeof meta.name !== "string") {
        throw new Error("`meta.name` must be a string");
    }
    if (typeof module.default !== "function") {
        throw new Error("the layout must export a default function that creates the layout");
    }

    const actions = meta.actions ?? {};
    const options = meta.options ?? {};
    for (const [name, spec] of Object.entries(options)) {
        if (!["boolean", "number", "string"].includes(spec.type)) {
            throw new Error(`option ${name} has an unknown type: ${spec.type}`);
        }
        if (typeof spec.default !== spec.type) {
            throw new Error(`the default of option ${name} must be a ${spec.type}`);
        }
    }

    layoutModule = module;
    instances.clear();
    return JSON.stringify({
        name: meta.name,
        actions: Object.keys(actions),
        options: Object.fromEntries(
            Object.entries(options).map(([name, spec]) => [name, spec.type]),
        ),
    });
}

/** Handles one event from bento, given and returned as JSON. */
export function handle(eventJson) {
    const event = JSON.parse(eventJson);
    if (event.type === "create") {
        const instance = new Instance(event);
        instances.set(event.instance, instance);
        return JSON.stringify(instance.create(event));
    }

    const instance = instances.get(event.instance);
    if (instance === undefined) {
        throw new Error(`no layout instance ${event.instance}`);
    }
    if (event.type === "destroy") {
        instances.delete(event.instance);
    }
    return JSON.stringify(instance.handle(event));
}

/**
 * Saves the state of every instance, before bento loads another version of the layout. Returns
 * the states by instance id, as JSON.
 */
export function saveAll() {
    const states = {};
    for (const [id, instance] of instances) {
        if (typeof instance.layout.save !== "function") {
            continue;
        }
        try {
            states[id] = instance.layout.save() ?? null;
        } catch (e) {
            // The layout starts from the window positions instead.
        }
    }
    return JSON.stringify(states);
}

/** Options with defaults filled in from the layout's `meta.options`, and types checked. */
function resolveOptions(given) {
    const specs = layoutModule.meta.options ?? {};
    const options = {};
    const errors = [];
    for (const [name, spec] of Object.entries(specs)) {
        options[name] = spec.default;
    }
    for (const [name, value] of Object.entries(given ?? {})) {
        const spec = specs[name];
        if (spec === undefined) {
            errors.push(`unknown layout option: ${name}`);
        } else if (typeof value !== spec.type) {
            errors.push(`layout option ${name} must be a ${spec.type}`);
        } else {
            options[name] = value;
        }
    }
    return { options: Object.freeze(options), errors };
}

function copyRect(rect) {
    return { x: rect.x, y: rect.y, w: rect.w, h: rect.h };
}

function sameRect(a, b) {
    return a !== null && b !== null && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}

/** Summary of a tab bar, to tell whether it changed. */
function tabBarKey(bar) {
    const { x, y, w, h } = bar.area;
    return [x, y, w, h, bar.active, ...bar.windows].join();
}

/**
 * The view as engines see it: what bento sends, plus `tilingArea`, the working area minus the
 * struts from the config, starting at a physical pixel.
 */
function layoutView(view, config) {
    const parent = view.workingArea;
    const struts = config.struts;
    const scale = view.scale;
    const x = parent.x + struts.left;
    const y = parent.y + struts.top;
    const w = Math.max(0, parent.w - struts.left - struts.right);
    const h = Math.max(0, parent.h - struts.top - struts.bottom);
    const px = Math.ceil(x * scale) / scale;
    const py = Math.ceil(y * scale) / scale;
    const tilingArea = Object.freeze({
        x: px,
        y: py,
        w: w - Math.min(w, px - x),
        h: h - Math.min(h, py - y),
    });
    return Object.freeze({ ...view, tilingArea });
}

function isRect(rect) {
    return (
        rect !== null &&
        typeof rect === "object" &&
        [rect.x, rect.y, rect.w, rect.h].every(Number.isFinite) &&
        rect.w >= 0 &&
        rect.h >= 0
    );
}

/** A window as layouts see it, with what bento reports about it and what the layout did to it. */
class LayoutWindow {
    constructor(info) {
        this.id = info.id;
        this.rect = null;
        this.visible = true;
        this.data = info.data ?? null;
        this.update(info);
    }

    update(info) {
        this.appId = info.appId;
        this.title = info.title;
        this.size = Object.freeze({ ...info.size });
        this.minSize = Object.freeze({ ...info.minSize });
        this.maxSize = Object.freeze({ ...info.maxSize });
        this.borders = Object.freeze({ ...info.borders });
        this.mode = info.mode;
        this.rules = Object.freeze({ ...info.rules });
        this.isUrgent = info.isUrgent;
    }
}

/** A layout for one workspace, with the `Workspace` it sees. */
class Instance {
    constructor(event) {
        this.id = event.instance;
        this.windows = [];
        this.byId = new Map();
        this.focusedId = null;
        this.camera = { ...event.camera };
        this.config = Object.freeze(event.config);
        this.view = layoutView(event.view, this.config);
        this.logs = [];
        this.setOptions(event.options);

        // What changed since the last result.
        this.dirty = new Set();
        this.animate = new Map();
        this.dataDirty = new Set();
        this.cameraChange = null;
        this.hint = null;
        this.hintChanged = false;
        this.orderChanged = false;
        this.focusChanged = false;
        this.tabBars = new Map();
        this.tabsChanged = false;

        this.ws = this.makeWorkspace();
        this.layout = layoutModule.default(this.ws);
        if (this.layout === null || typeof this.layout !== "object") {
            throw new Error("the layout's default export must return an object");
        }
        for (const required of ["windowAdded", "windowRemoved"]) {
            if (typeof this.layout[required] !== "function") {
                throw new Error(`the layout must have a ${required} method`);
            }
        }
    }

    setOptions(given) {
        const { options, errors } = resolveOptions(given);
        this.options = options;
        for (const error of errors) {
            this.logs.push(error);
        }
    }

    makeWorkspace() {
        const self = this;
        const own = (win) => {
            const found = win && self.byId.get(win.id);
            if (found !== win) {
                throw new Error("not a window in this layout");
            }
            return win;
        };
        const round = (value) => Math.round(value * self.view.scale) / self.view.scale;

        return Object.freeze({
            get windows() {
                return self.windows.slice();
            },
            get focused() {
                return self.focusedId === null ? null : self.byId.get(self.focusedId);
            },
            get view() {
                return self.view;
            },
            get camera() {
                return { ...self.camera };
            },
            get config() {
                return self.config;
            },
            get options() {
                return self.options;
            },
            window(id) {
                return self.byId.get(id) ?? null;
            },
            place(win, rect, opts = {}) {
                own(win);
                if (!isRect(rect)) {
                    throw new Error(`invalid rect for window ${win.id}: ${JSON.stringify(rect)}`);
                }
                if (sameRect(win.rect, rect)) {
                    return;
                }
                win.rect = Object.freeze(copyRect(rect));
                self.dirty.add(win.id);
                self.animate.set(win.id, opts.animate ?? true);
            },
            setWindowData(win, data) {
                own(win);
                // Check that it survives the trip through JSON.
                win.data = data === undefined ? null : JSON.parse(JSON.stringify(data));
                self.dataDirty.add(win.id);
            },
            setVisible(win, visible) {
                own(win);
                if (win.visible !== !!visible) {
                    win.visible = !!visible;
                    self.dirty.add(win.id);
                }
            },
            setMode(win, mode) {
                own(win);
                if (!["normal", "fullscreen", "maximized"].includes(mode)) {
                    throw new Error(`invalid sizing mode: ${mode}`);
                }
                if (win.mode !== mode) {
                    win.mode = mode;
                    self.dirty.add(win.id);
                }
            },
            raise(win) {
                own(win);
                const idx = self.windows.indexOf(win);
                if (idx !== self.windows.length - 1) {
                    self.windows.splice(idx, 1);
                    self.windows.push(win);
                    self.orderChanged = true;
                }
            },
            focus(win) {
                const id = win === null ? null : own(win).id;
                if (self.focusedId !== id) {
                    self.focusedId = id;
                    self.focusChanged = true;
                }
            },
            moveCamera(to, opts = {}) {
                const x = to.x ?? self.camera.x;
                const y = to.y ?? self.camera.y;
                if (!Number.isFinite(x) || !Number.isFinite(y)) {
                    throw new Error(`invalid camera position: ${JSON.stringify(to)}`);
                }
                self.camera = { x, y };
                self.cameraChange = { x, y, animate: opts.animate ?? true };
            },
            setHint(rect) {
                if (rect !== null && !isRect(rect)) {
                    throw new Error(`invalid hint rect: ${JSON.stringify(rect)}`);
                }
                if (rect === null ? self.hint === null : sameRect(self.hint, rect)) {
                    return;
                }
                self.hint = rect === null ? null : copyRect(rect);
                self.hintChanged = true;
            },
            setTabBar(key, bar) {
                key = String(key);
                if (bar === null || bar === undefined) {
                    if (self.tabBars.delete(key)) {
                        self.tabsChanged = true;
                    }
                    return;
                }
                if (!isRect(bar.area)) {
                    throw new Error(`invalid tab bar area: ${JSON.stringify(bar.area)}`);
                }
                const windows = bar.windows.map((win) => own(win).id);
                const active = own(bar.active).id;
                if (!windows.includes(active)) {
                    throw new Error("the active tab must be one of the tabs");
                }
                const next = { key, area: copyRect(bar.area), windows, active };
                const shown = self.tabBars.get(key);
                if (shown !== undefined && tabBarKey(shown) === tabBarKey(next)) {
                    return;
                }
                self.tabBars.set(key, next);
                self.tabsChanged = true;
            },
            get tabBars() {
                return [...self.tabBars.keys()];
            },
            tabIndicatorSpace(count) {
                const config = self.config.tabIndicator;
                if (
                    !config ||
                    config.off ||
                    !config.placeWithinColumn ||
                    (config.hideWhenSingleTab && count === 1)
                ) {
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
                self.logs.push(
                    args
                        .map((arg) => (typeof arg === "string" ? arg : JSON.stringify(arg)))
                        .join(" "),
                );
            },
        });
    }

    addWindow(info) {
        if (this.byId.has(info.id)) {
            throw new Error(`window ${info.id} is already in the layout`);
        }
        const win = new LayoutWindow(info);
        this.windows.push(win);
        this.byId.set(win.id, win);
        this.orderChanged = true;
        return win;
    }

    removeWindow(id) {
        const win = this.byId.get(id);
        if (win === undefined) {
            throw new Error(`window ${id} is not in the layout`);
        }
        this.byId.delete(id);
        this.windows.splice(this.windows.indexOf(win), 1);
        this.dirty.delete(id);
        this.dataDirty.delete(id);
        this.orderChanged = true;
        for (const [key, bar] of this.tabBars) {
            if (bar.windows.includes(id)) {
                const windows = bar.windows.filter((other) => other !== id);
                if (windows.length === 0) {
                    this.tabBars.delete(key);
                } else {
                    const active = bar.active === id ? windows[0] : bar.active;
                    this.tabBars.set(key, { ...bar, windows, active });
                }
                this.tabsChanged = true;
            }
        }
        if (this.focusedId === id) {
            this.focusedId = null;
            this.focusChanged = true;
        }
        return win;
    }

    mustWindow(id) {
        const win = this.byId.get(id);
        if (win === undefined) {
            throw new Error(`window ${id} is not in the layout`);
        }
        return win;
    }

    create(event) {
        const windows = event.windows.map((info) => {
            const win = this.addWindow(info);
            // The window is where it was; the layout moves it if it likes.
            win.rect = info.rect ? Object.freeze(copyRect(info.rect)) : null;
            return win;
        });
        const focused = event.focused === null ? null : this.byId.get(event.focused) ?? null;
        const hints = new Map(event.windows.map((info) => [info.id, info.rect ?? null]));
        return this.run(false, () => {
            if (typeof this.layout.adopt === "function") {
                this.layout.adopt(windows, focused, event.state ?? null);
            } else {
                for (const win of windows) {
                    this.layout.windowAdded(win, {
                        reason: "adopt",
                        activate: win === focused,
                        rect: hints.get(win.id),
                        pointer: null,
                        nextTo: null,
                    });
                }
            }
        });
    }

    handle(event) {
        const layout = this.layout;
        switch (event.type) {
            case "destroy":
                return this.run(true, () =>
                    typeof layout.save === "function" ? layout.save() : null,
                );
            case "add": {
                const win = this.addWindow(event.window);
                const how = event.how;
                return this.run(false, () =>
                    layout.windowAdded(win, {
                        reason: how.reason,
                        activate: how.activate,
                        rect: how.rect ?? null,
                        pointer: how.pointer ?? null,
                        nextTo: how.nextTo === null ? null : this.byId.get(how.nextTo) ?? null,
                    }),
                );
            }
            case "remove": {
                const win = this.removeWindow(event.id);
                return this.run(false, () => layout.windowRemoved(win, { reason: event.reason }));
            }
            case "change": {
                const wins = event.windows.map((info) => {
                    const win = this.mustWindow(info.id);
                    win.update(info);
                    return win;
                });
                return this.run(false, () => {
                    if (typeof layout.windowChanged === "function") {
                        for (const win of wins) {
                            layout.windowChanged(win);
                        }
                    }
                });
            }
            case "view":
                this.view = layoutView(event.view, this.config);
                return this.run(false, () => layout.viewChanged?.());
            case "config":
                this.config = Object.freeze(event.config);
                this.view = layoutView(this.view, this.config);
                this.setOptions(event.options);
                return this.run(false, () => layout.configChanged?.());
            case "camera":
                // bento moved the camera itself, for a gesture or while dragging near an edge.
                this.camera = { ...event.camera };
                return this.run(false, () => {});
            case "gesture-begin":
                return this.run(false, () => layout.gestureBegin?.());
            case "gesture-end":
                this.camera = { ...event.camera };
                return this.run(false, () =>
                    layout.gestureEnd?.({ ...event.velocity }, { ...event.projected }),
                );
            case "action": {
                const target = event.target === null ? null : this.mustWindow(event.target);
                return this.run(false, () =>
                    typeof layout.action === "function"
                        ? !!layout.action(event.name, event.args, target)
                        : false,
                );
            }
            case "activate": {
                const win = this.mustWindow(event.id);
                return this.run(false, () => {
                    if (typeof layout.activate === "function") {
                        layout.activate(win);
                    } else {
                        this.ws.focus(win);
                    }
                });
            }
            case "mode": {
                const win = this.mustWindow(event.id);
                return this.run(false, () => {
                    if (typeof layout.modeRequested === "function") {
                        layout.modeRequested(win, event.mode, { ...(event.request ?? {}) });
                    } else {
                        this.ws.setMode(win, event.mode);
                    }
                });
            }
            case "resize-begin": {
                const win = this.mustWindow(event.id);
                return this.run(false, () =>
                    typeof layout.resizeBegin === "function"
                        ? layout.resizeBegin(win, event.resize) !== false
                        : false,
                );
            }
            case "resize-update": {
                const win = this.mustWindow(event.id);
                return this.run(false, () => layout.resizeUpdate?.(win, event.resize));
            }
            case "resize-end": {
                const win = this.mustWindow(event.id);
                return this.run(false, () => layout.resizeEnd?.(win));
            }
            case "transfer-in": {
                const wins = event.windows.map((info) => this.addWindow(info));
                return this.run(false, () => {
                    if (typeof layout.transferIn === "function") {
                        layout.transferIn(wins, event.data ?? null, { activate: event.activate });
                    } else {
                        wins.forEach((win, i) =>
                            layout.windowAdded(win, {
                                reason: "move",
                                activate: event.activate && i === 0,
                                rect: null,
                                pointer: null,
                                nextTo: null,
                            }),
                        );
                    }
                });
            }
            case "transfer-out": {
                const win = this.mustWindow(event.id);
                return this.query(() => {
                    const transfer =
                        typeof layout.transferOut === "function"
                            ? layout.transferOut(win)
                            : { windows: [win] };
                    const ids = transfer.windows.map((w) => this.mustWindow(w.id).id);
                    if (!ids.includes(win.id)) {
                        throw new Error("transferOut must include the window itself");
                    }
                    return { windows: ids, data: transfer.data ?? null };
                });
            }
            case "drop-target": {
                return this.query(() => {
                    if (typeof layout.dropTarget !== "function") {
                        return null;
                    }
                    const win = new LayoutWindow(event.window);
                    const rect = layout.dropTarget(win, event.pointer, event.rect);
                    if (rect !== null && rect !== undefined && !isRect(rect)) {
                        throw new Error(`invalid drop target: ${JSON.stringify(rect)}`);
                    }
                    return rect ?? null;
                });
            }
            case "new-window-size": {
                return this.query(() => {
                    if (typeof layout.newWindowSize !== "function") {
                        return { w: 0, h: 0 };
                    }
                    const size = layout.newWindowSize(event.info);
                    if (!size || !Number.isFinite(size.w) || !Number.isFinite(size.h)) {
                        throw new Error(`invalid new window size: ${JSON.stringify(size)}`);
                    }
                    return { w: Math.max(0, size.w), h: Math.max(0, size.h) };
                });
            }
            default:
                throw new Error(`unknown event: ${event.type}`);
        }
    }

    /** Runs a call on the layout and returns what changed. */
    run(isQuery, call) {
        let result = null;
        let error = null;
        try {
            result = call() ?? null;
        } catch (e) {
            error = e instanceof Error ? `${e.message}\n${e.stack ?? ""}` : String(e);
        }
        if (isQuery) {
            return this.finish(result, error);
        }

        // Every window needs a place. Put windows the layout forgot in the middle of the view.
        for (const win of this.windows) {
            if (win.rect === null) {
                const area = this.view.workingArea;
                const w = Math.min(win.size.w || area.w / 2, area.w);
                const h = Math.min(win.size.h || area.h / 2, area.h);
                win.rect = Object.freeze({
                    x: this.camera.x + area.x + (area.w - w) / 2,
                    y: this.camera.y + area.y + (area.h - h) / 2,
                    w,
                    h,
                });
                this.dirty.add(win.id);
                this.logs.push(`the layout didn't place window ${win.id}`);
            }
        }

        return this.finish(result, error);
    }

    /** Runs a query, undoing anything it changed in the workspace. */
    query(call) {
        const saved = {
            windows: this.windows.slice(),
            rects: this.windows.map((win) => [win, win.rect, win.visible, win.mode, win.data]),
            focusedId: this.focusedId,
            camera: this.camera,
            hint: this.hint,
            tabBars: new Map(this.tabBars),
        };
        let result = null;
        let error = null;
        try {
            result = call() ?? null;
        } catch (e) {
            error = e instanceof Error ? `${e.message}\n${e.stack ?? ""}` : String(e);
        }

        this.windows = saved.windows;
        for (const [win, rect, visible, mode, data] of saved.rects) {
            win.rect = rect;
            win.visible = visible;
            win.mode = mode;
            win.data = data;
        }
        this.focusedId = saved.focusedId;
        this.camera = saved.camera;
        this.hint = saved.hint;
        this.tabBars = saved.tabBars;
        this.tabsChanged = false;
        this.dirty.clear();
        this.animate.clear();
        this.dataDirty.clear();
        this.cameraChange = null;
        this.hintChanged = false;
        this.orderChanged = false;
        this.focusChanged = false;

        return { result, error, logs: this.takeLogs() };
    }

    finish(result, error) {
        const out = { result, error, logs: this.takeLogs() };

        if (this.dirty.size > 0) {
            out.windows = [...this.dirty].map((id) => {
                const win = this.byId.get(id);
                return {
                    id,
                    rect: win.rect,
                    visible: win.visible,
                    mode: win.mode,
                    animate: this.animate.get(id) ?? true,
                };
            });
        }
        if (this.orderChanged) {
            out.order = this.windows.map((win) => win.id);
        }
        if (this.focusChanged) {
            out.focused = this.focusedId;
        }
        if (this.cameraChange !== null) {
            out.camera = this.cameraChange;
        }
        if (this.hintChanged) {
            out.hint = this.hint;
        }
        if (this.tabsChanged) {
            out.tabs = [...this.tabBars.values()];
        }
        if (this.dataDirty.size > 0) {
            out.data = [...this.dataDirty].map((id) => [id, this.byId.get(id).data]);
        }
        if (typeof this.layout?.ipcPosition === "function" && (this.dirty.size > 0 || this.orderChanged)) {
            try {
                out.ipc = this.windows.map((win) => [win.id, this.layout.ipcPosition(win) ?? null]);
            } catch (e) {
                out.logs.push(`ipcPosition failed: ${e}`);
            }
        }

        this.dirty.clear();
        this.animate.clear();
        this.dataDirty.clear();
        this.cameraChange = null;
        this.hintChanged = false;
        this.orderChanged = false;
        this.focusChanged = false;
        this.tabsChanged = false;
        return out;
    }

    takeLogs() {
        const logs = this.logs;
        this.logs = [];
        return logs;
    }
}
