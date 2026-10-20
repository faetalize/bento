// A stand-in for bento, to test layouts with node:
//
//     import { TestHost } from "./host.js";
//     const host = await TestHost.load("../bento.js", { options: { gravity: true } });
//     const a = host.open();
//     host.action("focus-column-left");
//     assert.deepEqual(host.rect(a), { x: 16, y: 16, w: 616, h: 688 });
//
// It talks to the layout through `prelude.js` with the same JSON events as bento, and windows
// behave like simple clients: they take the size they're given, within their size limits.

import * as prelude from "../prelude.js";

const DEFAULT_CONFIG = {
    gaps: 16,
    struts: { left: 0, right: 0, top: 0, bottom: 0 },
    centerFocusedColumn: "never",
    alwaysCenterSingleColumn: false,
    defaultColumnWidth: { proportion: 0.5 },
    presetColumnWidths: [{ proportion: 1 / 3 }, { proportion: 0.5 }, { proportion: 2 / 3 }],
    presetWindowHeights: [{ proportion: 1 / 3 }, { proportion: 0.5 }, { proportion: 2 / 3 }],
    defaultColumnDisplay: "normal",
    tabIndicator: {
        off: false,
        position: "left",
        width: 4,
        gap: 5,
        placeWithinColumn: false,
        hideWhenSingleTab: false,
    },
};

export class TestHost {
    /**
     * Loads a layout module and creates it for an empty workspace.
     *
     * `opts.view` is the output size, 1280 × 720 by default; `opts.workingArea` defaults to all of
     * it. `opts.config` overrides parts of the `layout {}` config.
     */
    static async load(path, opts = {}) {
        const url = new URL(path, import.meta.url);
        const module = await import(url.href);
        return new TestHost(module, opts);
    }

    constructor(module, opts = {}) {
        this.meta = JSON.parse(prelude.load(module));
        const size = opts.view ?? { w: 1280, h: 720 };
        this.view = {
            size,
            workingArea: opts.workingArea ?? { x: 0, y: 0, w: size.w, h: size.h },
            scale: opts.scale ?? 1,
        };
        this.config = { ...DEFAULT_CONFIG, ...(opts.config ?? {}) };
        this.options = opts.options ?? {};
        this.instance = TestHost.nextInstance++;
        this.nextId = 1;
        this.windows = new Map();
        this.order = [];
        this.focused = null;
        this.camera = { x: 0, y: 0 };
        this.hint = null;
        /** Tab bars by key, with window ids. */
        this.tabs = new Map();
        this.ipc = new Map();
        this.logs = [];
        this.errors = [];

        this.send({
            type: "create",
            view: this.view,
            camera: this.camera,
            config: this.config,
            options: this.options,
            windows: [],
            focused: null,
            state: opts.state ?? null,
        });
    }

    /**
     * Opens a window. `opts` can set `minSize`, `maxSize`, `borders`, `appId`, `title`,
     * `defaultWidth` and `defaultHeight` (presets), `activate` (default true) and `nextTo`.
     */
    open(opts = {}) {
        const info = this.makeWindow(opts);
        this.send({
            type: "add",
            window: this.info(info),
            how: {
                reason: "open",
                activate: opts.activate ?? true,
                rect: null,
                pointer: null,
                nextTo: opts.nextTo ?? null,
            },
        });
        return info.id;
    }

    makeWindow(opts) {
        const id = this.nextId++;
        const win = {
            id,
            appId: opts.appId ?? null,
            title: opts.title ?? null,
            size: { w: 0, h: 0 },
            minSize: opts.minSize ?? { w: 0, h: 0 },
            maxSize: opts.maxSize ?? { w: 0, h: 0 },
            borders: opts.borders ?? { w: 0, h: 0 },
            mode: "normal",
            rules: {
                defaultWidth: opts.defaultWidth ?? this.config.defaultColumnWidth,
                defaultHeight: opts.defaultHeight ?? null,
            },
            isUrgent: false,
            rect: null,
            visible: true,
        };
        this.windows.set(id, win);
        return win;
    }

    close(id) {
        this.forget(id);
        this.send({ type: "remove", id, reason: "close" });
    }

    forget(id) {
        this.windows.delete(id);
        this.order = this.order.filter((other) => other !== id);
        this.ipc.delete(id);
        if (this.focused === id) {
            this.focused = null;
        }
    }

    /** Runs a built-in or layout action. Returns whether the layout did anything. */
    action(name, ...args) {
        return this.send({ type: "action", name, args, target: null }).result;
    }

    /** Runs an action on a specific window, like `set-window-width id=...`. */
    actionOn(id, name, ...args) {
        return this.send({ type: "action", name, args, target: id }).result;
    }

    activate(id) {
        this.send({ type: "activate", id });
    }

    /** Clicks a tab in a tab bar, which activates its window. */
    clickTab(key, index) {
        this.activate(this.tabs.get(key).windows[index]);
    }

    requestMode(id, mode) {
        this.send({ type: "mode", id, mode });
    }

    /** Changes a window's size limits, like a client committing new ones. */
    setLimits(id, { minSize, maxSize }) {
        const win = this.windows.get(id);
        if (minSize) win.minSize = minSize;
        if (maxSize) win.maxSize = maxSize;
        this.settleSize(win);
        this.send({ type: "change", windows: [this.info(win)] });
    }

    resizeView({ size, workingArea, scale }) {
        if (size) this.view.size = size;
        this.view.workingArea = workingArea ?? { x: 0, y: 0, w: this.view.size.w, h: this.view.size.h };
        if (scale) this.view.scale = scale;
        this.send({ type: "view", view: this.view });
    }

    setOptions(options) {
        this.options = options;
        this.send({ type: "config", config: this.config, options });
    }

    /**
     * Moves the camera like a swipe gesture by `dx`, then ends the gesture. `projected` is where
     * the camera would coast to, by default where the swipe left it.
     */
    swipe(dx, velocity = { x: 0, y: 0 }, projected = null) {
        this.camera = { x: this.camera.x + dx, y: this.camera.y };
        this.send({
            type: "gesture-end",
            camera: this.camera,
            velocity,
            projected: projected ?? { ...this.camera },
        });
    }

    /** Where a window dragged with the pointer at `pointer` would land, without dropping it. */
    dropTarget(id, pointer, rect) {
        const win = this.windows.get(id);
        return this.send({ type: "drop-target", window: this.info(win), pointer, rect }).result;
    }

    /**
     * Drags a window and drops it: it leaves the layout, then comes back where it was dropped.
     * `rect` is where the dragged tile is on the canvas when it's dropped.
     */
    drag(id, pointer, rect) {
        const win = this.windows.get(id);
        this.send({ type: "remove", id, reason: "drag" });
        this.order = this.order.filter((other) => other !== id);
        this.send({
            type: "add",
            window: this.info(win),
            how: { reason: "drop", activate: true, rect, pointer, nextTo: null },
        });
    }

    resize(id, edges, delta) {
        const resize = {
            edges: { left: false, right: false, top: false, bottom: false, ...edges },
            delta,
        };
        const began = this.send({ type: "resize-begin", id, resize }).result;
        if (!began) {
            return false;
        }
        this.send({ type: "resize-update", id, resize });
        this.send({ type: "resize-end", id });
        return true;
    }

    /** Asks the layout which windows go along with `id` to another workspace. */
    transferOut(id) {
        return this.send({ type: "transfer-out", id }).result;
    }

    newWindowSize(info = {}) {
        return this.send({
            type: "new-window-size",
            info: {
                rules: { defaultWidth: this.config.defaultColumnWidth, defaultHeight: null, ...(info.rules ?? {}) },
                minSize: info.minSize ?? { w: 0, h: 0 },
                maxSize: info.maxSize ?? { w: 0, h: 0 },
                borders: info.borders ?? { w: 0, h: 0 },
            },
        }).result;
    }

    /** Saves the layout's state and creates the layout again, like reloading the layout file. */
    reload() {
        const state = this.send({ type: "destroy" }).result;
        this.instance = TestHost.nextInstance++;
        this.send({
            type: "create",
            view: this.view,
            camera: this.camera,
            config: this.config,
            options: this.options,
            windows: this.order.map((id) => this.info(this.windows.get(id))),
            focused: this.focused,
            state,
        });
    }

    rect(id) {
        return this.windows.get(id).rect;
    }

    /** Rect of a window relative to the view, which is what's on screen. */
    screenRect(id) {
        const rect = this.rect(id);
        return { x: rect.x - this.camera.x, y: rect.y - this.camera.y, w: rect.w, h: rect.h };
    }

    isVisible(id) {
        return this.windows.get(id).visible;
    }

    mode(id) {
        return this.windows.get(id).mode;
    }

    /** One line per window in the order they opened, for snapshot-style asserts. */
    format() {
        return [...this.windows.values()]
            .map((win) => {
                const r = win.rect;
                const flags = [
                    win.id === this.focused ? "focused" : null,
                    win.visible ? null : "hidden",
                    win.mode === "normal" ? null : win.mode,
                ].filter(Boolean);
                return `${win.id}: ${r.w}×${r.h} at ${r.x},${r.y}${flags.length ? " " + flags.join(" ") : ""}`;
            })
            .join("\n");
    }

    info(win) {
        const { rect, visible, ...info } = win;
        return { ...info, rect };
    }

    send(event) {
        const out = JSON.parse(prelude.handle(JSON.stringify({ instance: this.instance, ...event })));
        this.logs.push(...out.logs);
        if (out.error !== null) {
            this.errors.push(out.error);
            throw new Error(`layout error: ${out.error}`);
        }

        for (const change of out.windows ?? []) {
            const win = this.windows.get(change.id);
            win.rect = change.rect;
            win.visible = change.visible;
            win.mode = change.mode;
            this.settleSize(win);
        }
        if (out.order) this.order = out.order;
        if ("focused" in out) this.focused = out.focused;
        if (out.camera) this.camera = { x: out.camera.x, y: out.camera.y };
        if ("hint" in out) this.hint = out.hint;
        if (out.tabs) this.tabs = new Map(out.tabs.map((bar) => [bar.key, bar]));
        for (const [id, pos] of out.ipc ?? []) this.ipc.set(id, pos);

        // Windows that couldn't take their size tell the layout, like clients committing.
        const changed = [...this.windows.values()].filter((win) => win.reported !== sizeKey(win.size));
        for (const win of changed) win.reported = sizeKey(win.size);
        const resized = changed.filter((win) => win.rect && (win.rect.w !== win.size.w || win.rect.h !== win.size.h));
        if (resized.length > 0 && event.type !== "change") {
            this.send({ type: "change", windows: resized.map((win) => this.info(win)) });
        }
        return out;
    }

    /** Gives a window the size it was placed at, within its limits. */
    settleSize(win) {
        if (!win.rect) return;
        if (win.mode === "fullscreen") {
            win.size = { ...this.view.size };
            return;
        }
        const clamp = (v, min, max) => Math.max(min, max > 0 ? Math.min(v, max) : v);
        win.size = {
            w: clamp(win.rect.w, win.minSize.w, win.maxSize.w),
            h: clamp(win.rect.h, win.minSize.h, win.maxSize.h),
        };
    }
}

TestHost.nextInstance = 1;

function sizeKey(size) {
    return `${size.w}x${size.h}`;
}
