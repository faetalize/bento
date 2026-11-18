// A desktop engine: overlapping windows that snap to halves and quarters, the way Windows 11
// manages windows.
//
// Every window floats. It opens at the size it asks for, centred in the working area, a step
// down and to the right of a window already there, and stays where it's put. Focusing a window
// raises it. Dragging a window to the left or right edge of the screen snaps it to that half, to
// a corner to that quarter, and to the top edge maximizes it; Mod and the arrows do the same from
// the keyboard. Windows snapped side by side share their edge: resizing one across it resizes
// the others along it. After a window snaps, Snap Assist offers the other windows to fill the
// space left. Alt+Tab switches windows, most recently used first, and windows can be minimized.
//
// The engine keeps, for each window:
// - `free`, the rect it has when it's neither snapped nor maximized, which it goes back to;
// - `zone`, where it's snapped, as fractions of the working area: `{ x: 0, y: 0, w: 0.5, h: 1 }`
//   is the left half, or null;
// - `minimized`, whether it's hidden.
// Maximized and fullscreen are the window's mode, so a snapped window that's maximized goes back
// to its zone. The state lives in each window's data, so it survives drags and reloads.
//
// bento's floating layer is off, since every window is free here: dialogs come as
// `floatrequest` and are centred over their parent.
//
// Coordinates: the camera stays at the canvas origin with no zoom, so canvas coordinates are view
// coordinates.
//
// What's drawn (the snap preview, Snap Assist, the window switcher and the layouts picker) goes
// in one layer per workspace, kept above the windows. The switcher's thumbnails are live copies of
// the windows, which stay where they are behind it. Snap Assist's are the windows themselves:
// their nodes move into what's drawn, scaled down, and the one picked grows from its thumbnail
// into its zone, while the others go back to their places.

export const meta = {
    api: 2,
    name: "windows11",
    description: "Overlapping windows that snap to halves and quarters, like Windows 11.",
    // Every window is free, so there's no second layer for floating ones.
    floating: false,
    minimize: true,
    // Windows move with the pointer from the first pixel, and the view never scrolls.
    freeDrag: true,
    actions: {
        "snap-left": {
            description:
                "Snap the focused window to the left half. From the right half it goes back to its own size, and from the left half to the monitor on the left.",
        },
        "snap-right": {
            description:
                "Snap the focused window to the right half. From the left half it goes back to its own size, and from the right half to the monitor on the right.",
        },
        "snap-up": {
            description:
                "Maximize the focused window, or snap it from a half to the top quarter of that side.",
        },
        "snap-down": {
            description:
                "Restore a maximized window, snap a half to its bottom quarter, or minimize the focused window.",
        },
        "switch-windows": {
            description:
                "Switch between the workspace's windows, most recently used first. Letting go of the keys held picks the highlighted one.",
        },
        "switch-windows-back": {
            description: "Switch between the workspace's windows, from the least recently used.",
        },
        "show-layouts": {
            description: "Show the snap layouts, to snap the focused window into a zone of one.",
        },
        minimize: { description: "Minimize the focused window." },
        "show-desktop": {
            description: "Minimize every window on the workspace, or bring them back.",
        },
        "move-to-monitor-left": {
            description: "Move the focused window to the monitor on the left, keeping its place.",
        },
        "move-to-monitor-right": {
            description: "Move the focused window to the monitor on the right, keeping its place.",
        },
    },
    options: {
        "snap-assist": {
            type: "boolean",
            default: true,
            description: "After a window snaps, offer the other windows to fill the space left.",
        },
        "edge-snap": {
            type: "boolean",
            default: true,
            description: "Snap windows dragged to the edges and corners of the screen.",
        },
    },
};

// Zones, as fractions of the working area.
const LEFT = { x: 0, y: 0, w: 0.5, h: 1 };
const RIGHT = { x: 0.5, y: 0, w: 0.5, h: 1 };
const TOP_LEFT = { x: 0, y: 0, w: 0.5, h: 0.5 };
const TOP_RIGHT = { x: 0.5, y: 0, w: 0.5, h: 0.5 };
const BOTTOM_LEFT = { x: 0, y: 0.5, w: 0.5, h: 0.5 };
const BOTTOM_RIGHT = { x: 0.5, y: 0.5, w: 0.5, h: 0.5 };
/** The drop target of the top edge, which maximizes. */
const MAXIMIZE = "maximize";

/** The layouts that `show-layouts` offers, each a list of zones. */
const LAYOUTS = [
    [LEFT, RIGHT],
    [
        { x: 0, y: 0, w: 2 / 3, h: 1 },
        { x: 2 / 3, y: 0, w: 1 / 3, h: 1 },
    ],
    [
        { x: 0, y: 0, w: 1 / 3, h: 1 },
        { x: 1 / 3, y: 0, w: 1 / 3, h: 1 },
        { x: 2 / 3, y: 0, w: 1 / 3, h: 1 },
    ],
    [TOP_LEFT, TOP_RIGHT, BOTTOM_LEFT, BOTTOM_RIGHT],
];

/** How close to an edge of the working area the pointer snaps a dragged window. */
const EDGE_REACH = 24;
/** How far along an edge from a corner the pointer snaps to that quarter. */
const CORNER_REACH = 120;
/** How far down and right a new window opens from one already at its place. */
const CASCADE_STEP = 32;
/** Smallest snapped window, in logical pixels, when resizing across a shared edge. */
const MIN_SNAPPED = 160;
/** Tolerance for comparing fractions of the working area. */
const EPSILON = 1e-3;

// Looks: soft, neutral and translucent, readable on light and dark wallpapers.
const PANEL = "#202020e0";
const PANEL_EDGE = "#ffffff1f";
const HIGHLIGHT = "#ffffff1f";
const HIGHLIGHT_EDGE = "#ffffffb3";
const TEXT = "#ffffffe6";
const FONT = "sans 13";
const PREVIEW = "#c8c8c84d";
const PREVIEW_EDGE = "#ffffff8c";
const ZONE = "#ffffff33";
const ZONE_PICKED = "#ffffffbf";
const RADIUS = 12;
/** How what's drawn moves and fades: quickly, without overshooting. */
const MOTION = { duration: 180, curve: "ease-out-cubic" };
/**
 * Window nodes turn into thumbnails with `MOTION`. Without a `rect` transition, they move and
 * resize with bento's own animations.
 */
const WINDOW_TRANSITIONS = { transform: MOTION };
const IDENTITY = { scale: 1, x: 0, y: 0 };

// Sizes of what's drawn.
const OUTER = 16;
const PAD = 10;
const CARD_GAP = 12;
const TITLE = 22;
/** The corners of a copied window in the switcher, smaller than its card's, which it sits in. */
const THUMB_RADIUS = 6;

const clamp = (value, min, max) => Math.min(Math.max(value, min), max);
const near = (a, b) => Math.abs(a - b) < EPSILON;
const isRect = (r) =>
    r !== null && typeof r === "object" && [r.x, r.y, r.w, r.h].every(Number.isFinite);
const grow = (r, by) => ({ x: r.x - by, y: r.y - by, w: r.w + 2 * by, h: r.h + 2 * by });

/** The same zone on the other side, left to right. */
const mirror = (z) => ({ ...z, x: 1 - z.x - z.w });

export default function create(bento) {
    /** Per window id: `{ free, zone, minimized }`. */
    const states = new Map();
    /** Window ids, most recently focused first. */
    const recent = [];
    /** Per workspace id: the windows `show-desktop` hid, and the one that had focus. */
    const desktops = new Map();
    /** Per workspace id: the layer for what's drawn, above the windows. */
    const layers = new Map();
    /** Per workspace id: the snap preview while a window is dragged, and the timer removing it. */
    const previews = new Map();
    /** Per workspace id: where its camera was when the engine started, for adopted windows. */
    const cameraShift = new Map();
    /** What's shown and has the keyboard: the switcher, Snap Assist or the layouts picker. */
    let ui = null;
    /** The resize going on: the window, and at its start its rect and the snapped zones. */
    let resizing = null;

    const saved = bento.savedState;
    if (saved !== null && typeof saved === "object") {
        if (Array.isArray(saved.recent)) recent.push(...saved.recent.filter(Number.isInteger));
        for (const [id, desktop] of saved.desktops ?? []) desktops.set(id, desktop);
    }

    // ---------------------------------------------------------------------------------------------
    // State
    // ---------------------------------------------------------------------------------------------

    function stateOf(win) {
        let s = states.get(win.id);
        if (s === undefined) {
            const data = win.data ?? {};
            s = {
                free: isRect(data.free) ? { ...data.free } : null,
                zone: isRect(data.zone) ? { ...data.zone } : null,
                minimized: data.minimized === true,
            };
            states.set(win.id, s);
        }
        return s;
    }

    function save(win) {
        const s = stateOf(win);
        win.setData({ free: s.free, zone: s.zone, minimized: s.minimized });
        bento.stateChanged();
    }

    /** Snapped to a zone and shown there: not maximized, fullscreen or minimized. */
    function isSnapped(win) {
        return stateOf(win).zone !== null && win.mode === "normal" && win.visible;
    }

    function isFree(win) {
        return stateOf(win).zone === null && win.mode === "normal";
    }

    function touch(win) {
        const at = recent.indexOf(win.id);
        if (at !== -1) recent.splice(at, 1);
        recent.unshift(win.id);
        bento.stateChanged();
    }

    function forget(win) {
        const at = recent.indexOf(win.id);
        if (at !== -1) recent.splice(at, 1);
        states.delete(win.id);
        for (const desktop of desktops.values()) {
            desktop.ids = desktop.ids.filter((id) => id !== win.id);
        }
    }

    /** A workspace's windows, most recently used first. */
    function byRecent(ws) {
        const rank = (win) => {
            const at = recent.indexOf(win.id);
            return at === -1 ? Infinity : at;
        };
        // Windows that were never focused come after, the topmost first.
        const windows = ws.windows.slice().reverse();
        return windows.sort((a, b) => rank(a) - rank(b));
    }

    // ---------------------------------------------------------------------------------------------
    // Geometry
    // ---------------------------------------------------------------------------------------------

    /** The part of the working area that zones divide, so that gaps go around each zone. */
    function snapSpan(ws) {
        const a = ws.tilingArea;
        const g = ws.config.gaps;
        return { x: a.x + g / 2, y: a.y + g / 2, w: Math.max(1, a.w - g), h: Math.max(1, a.h - g) };
    }

    function zoneRect(ws, zone) {
        const s = snapSpan(ws);
        const g = ws.config.gaps / 2;
        const m = ws.monitor;
        const left = m.round(s.x + zone.x * s.w + g);
        const right = m.round(s.x + (zone.x + zone.w) * s.w - g);
        const top = m.round(s.y + zone.y * s.h + g);
        const bottom = m.round(s.y + (zone.y + zone.h) * s.h - g);
        return { x: left, y: top, w: Math.max(1, right - left), h: Math.max(1, bottom - top) };
    }

    /** Where a window goes, from its mode and state. */
    function targetRect(win) {
        const ws = win.workspace;
        const s = stateOf(win);
        if (win.mode === "fullscreen") return { x: 0, y: 0, ...ws.monitor.size };
        if (win.mode === "maximized") return { ...ws.monitor.workingArea };
        if (s.zone !== null) return zoneRect(ws, s.zone);
        return s.free;
    }

    function apply(win, opts) {
        const rect = targetRect(win);
        if (rect !== null) win.place(rect, opts);
    }

    /** The size a window opens at when it doesn't say. */
    function defaultSize(ws) {
        const a = ws.tilingArea;
        return { w: Math.round(a.w * 0.6), h: Math.round(a.h * 0.7) };
    }

    /** `size`, made to fit the working area with a gap around it. */
    function fitSize(ws, size) {
        const a = ws.tilingArea;
        const g = ws.config.gaps;
        return {
            w: Math.max(1, Math.min(size.w, a.w - 2 * g)),
            h: Math.max(1, Math.min(size.h, a.h - 2 * g)),
        };
    }

    /** Keeps a free rect's top edge in the working area and part of it on screen. */
    function reachable(ws, rect) {
        const a = ws.tilingArea;
        const m = ws.monitor;
        const keep = Math.min(rect.w, 64);
        return {
            x: m.round(clamp(rect.x, a.x - rect.w + keep, a.x + a.w - keep)),
            y: m.round(clamp(rect.y, a.y, Math.max(a.y, a.y + a.h - 48))),
            w: rect.w,
            h: rect.h,
        };
    }

    /**
     * A rect of `size` centred in the working area, a step down and to the right of each free
     * window already at that place, starting again from the top left when it runs out of room.
     */
    function cascade(ws, size, except) {
        const a = ws.tilingArea;
        const g = ws.config.gaps;
        const m = ws.monitor;
        const { w, h } = fitSize(ws, size);
        const taken = (at) =>
            ws.windows.some(
                (other) =>
                    other !== except &&
                    other.visible &&
                    other.rect !== null &&
                    isFree(other) &&
                    Math.abs(other.rect.x - at.x) < 1 &&
                    Math.abs(other.rect.y - at.y) < 1,
            );
        let at = { x: m.round(a.x + (a.w - w) / 2), y: m.round(a.y + (a.h - h) / 2) };
        for (let i = 0; i < 64 && taken(at); i++) {
            at = { x: at.x + CASCADE_STEP, y: at.y + CASCADE_STEP };
            if (at.x + w > a.x + a.w - g || at.y + h > a.y + a.h - g) {
                at = { x: a.x + g, y: a.y + g };
            }
        }
        return { ...at, w, h };
    }

    /** A rect of `size` centred over `parent`, kept on screen. */
    function centredOver(parent, size) {
        const ws = parent.workspace;
        const p = parent.rect ?? targetRect(parent);
        const { w, h } = fitSize(ws, size);
        const a = ws.tilingArea;
        const m = ws.monitor;
        return {
            x: m.round(clamp(p.x + (p.w - w) / 2, a.x, a.x + a.w - w)),
            y: m.round(clamp(p.y + (p.h - h) / 2, a.y, a.y + a.h - h)),
            w,
            h,
        };
    }

    /** A free rect on `from`, at the same place relative to the working area on `to`. */
    function mapRect(rect, from, to) {
        const fa = from.tilingArea;
        const ta = to.tilingArea;
        const size = fitSize(to, rect);
        return reachable(to, {
            x: ta.x + ((rect.x - fa.x) * ta.w) / Math.max(1, fa.w),
            y: ta.y + ((rect.y - fa.y) * ta.h) / Math.max(1, fa.h),
            ...size,
        });
    }

    /** What a window dropped with the pointer at `p` snaps to: a zone, `MAXIMIZE`, or null. */
    function dropTarget(ws, p) {
        if (!ws.options["edge-snap"]) return null;
        const a = ws.monitor.workingArea;
        const nearTop = p.y < a.y + CORNER_REACH;
        const nearBottom = p.y > a.y + a.h - CORNER_REACH;
        const nearLeft = p.x < a.x + CORNER_REACH;
        const nearRight = p.x > a.x + a.w - CORNER_REACH;
        if (p.x < a.x + EDGE_REACH) {
            return nearTop ? TOP_LEFT : nearBottom ? BOTTOM_LEFT : LEFT;
        }
        if (p.x > a.x + a.w - EDGE_REACH) {
            return nearTop ? TOP_RIGHT : nearBottom ? BOTTOM_RIGHT : RIGHT;
        }
        if (p.y < a.y + EDGE_REACH) {
            return nearLeft ? TOP_LEFT : nearRight ? TOP_RIGHT : MAXIMIZE;
        }
        if (p.y > a.y + a.h - EDGE_REACH) {
            return nearLeft ? BOTTOM_LEFT : nearRight ? BOTTOM_RIGHT : null;
        }
        return null;
    }

    /**
     * The largest part of the working area that no snapped window covers, as a zone, or null.
     * Between parts of the same size, the taller one: the column beside a quarter rather than the
     * row below it.
     */
    function freeZone(ws) {
        const zones = ws.windows.filter(isSnapped).map((win) => stateOf(win).zone);
        const lines = (start, size) => {
            const all = [0, 1, ...zones.flatMap((z) => [z[start], z[start] + z[size]])];
            const sorted = all.map((v) => clamp(v, 0, 1)).sort((a, b) => a - b);
            return sorted.filter((v, i) => i === 0 || !near(v, sorted[i - 1]));
        };
        const xs = lines("x", "w");
        const ys = lines("y", "h");
        const covered = (x, y) =>
            zones.some((z) => z.x < x && x < z.x + z.w && z.y < y && y < z.y + z.h);
        const free = (i0, i1, j0, j1) => {
            for (let i = i0; i < i1; i++) {
                for (let j = j0; j < j1; j++) {
                    if (covered((xs[i] + xs[i + 1]) / 2, (ys[j] + ys[j + 1]) / 2)) return false;
                }
            }
            return true;
        };
        let best = null;
        for (let i0 = 0; i0 < xs.length - 1; i0++) {
            for (let i1 = i0 + 1; i1 < xs.length; i1++) {
                for (let j0 = 0; j0 < ys.length - 1; j0++) {
                    for (let j1 = j0 + 1; j1 < ys.length; j1++) {
                        if (!free(i0, i1, j0, j1)) continue;
                        const z = { x: xs[i0], y: ys[j0], w: xs[i1] - xs[i0], h: ys[j1] - ys[j0] };
                        const area = z.w * z.h;
                        const bestArea = best === null ? 0 : best.w * best.h;
                        if (area > bestArea + EPSILON || (near(area, bestArea) && z.h > best.h)) {
                            best = z;
                        }
                    }
                }
            }
        }
        // A sliver isn't worth offering windows for.
        return best !== null && best.w * best.h >= 0.1 ? best : null;
    }

    // ---------------------------------------------------------------------------------------------
    // Workspaces and what's drawn on them
    // ---------------------------------------------------------------------------------------------

    const tracked = new Set();

    function track(ws) {
        if (tracked.has(ws.id)) return;
        tracked.add(ws.id);
        const camera = ws.camera;
        cameraShift.set(ws.id, { x: camera.x, y: camera.y });
        if (camera.x !== 0 || camera.y !== 0 || camera.zoom !== 1) {
            camera.move({ x: 0, y: 0, zoom: 1 }, { animate: false });
        }
        ws.addEventListener("dragenter", (e) => dragOver(ws, e));
        ws.addEventListener("dragmove", (e) => dragOver(ws, e));
        ws.addEventListener("dragleave", () => hidePreview(ws));
        ws.addEventListener("change", () => ws.windows.forEach((win) => apply(win)));
        ws.addEventListener("remove", () => {
            if (ui !== null && ui.ws === ws) closeUi();
            tracked.delete(ws.id);
            layers.delete(ws.id);
            previews.delete(ws.id);
            desktops.delete(ws.id);
            cameraShift.delete(ws.id);
        });
    }

    function layerOf(ws) {
        let layer = layers.get(ws.id);
        if (layer === undefined) {
            layer = bento.createNode("box");
            layers.set(ws.id, layer);
        }
        if (ws.tree.children.at(-1) !== layer) ws.tree.append(layer);
        return layer;
    }

    /** Keeps what's drawn above windows that were raised or opened since. */
    function raiseLayer(ws) {
        const layer = layers.get(ws.id);
        if (layer !== undefined && ws.tree.children.at(-1) !== layer) ws.tree.append(layer);
    }

    /** Draws a window above the others, unless it's shown as a thumbnail. */
    function raise(win) {
        const ws = win.workspace;
        if (win.node.parent === ws.tree) {
            win.raise();
            raiseLayer(ws);
        }
    }

    /** Draws the snap preview at `rect`, growing from `from` when it wasn't showing. */
    function showPreview(ws, rect, from) {
        let preview = previews.get(ws.id);
        if (preview?.timer != null) {
            bento.clearTimeout(preview.timer);
            preview.timer = null;
        }
        if (preview === undefined || preview.node.parent === null) {
            const node = bento.createNode("box", {
                rect,
                opacity: 1,
                cornerRadius: RADIUS,
                background: PREVIEW,
                border: { width: 1, color: PREVIEW_EDGE },
                transitions: { rect: MOTION, opacity: MOTION },
                enter: { opacity: 0, rect: from ?? rect },
            });
            preview = { node, timer: null };
            previews.set(ws.id, preview);
            layerOf(ws).append(node);
        } else {
            preview.node.set({ rect, opacity: 1 });
        }
    }

    /** Fades the snap preview out, and takes it away once it has. */
    function hidePreview(ws) {
        const preview = previews.get(ws.id);
        if (preview === undefined || preview.node.parent === null || preview.timer !== null) {
            return;
        }
        preview.node.set({ opacity: 0 });
        preview.timer = bento.setTimeout(() => {
            preview.timer = null;
            preview.node.remove();
        }, MOTION.duration);
    }

    function dragOver(ws, e) {
        const target = dropTarget(ws, e.pointer);
        if (target === null) {
            hidePreview(ws);
        } else {
            const rect = target === MAXIMIZE ? { ...ws.monitor.workingArea } : zoneRect(ws, target);
            showPreview(ws, rect, e.rect);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Focus, raising and minimizing
    // ---------------------------------------------------------------------------------------------

    /** Focuses a window and raises it, bringing it back first if it's minimized. */
    function activate(win) {
        if (stateOf(win).minimized || !win.visible) unminimize(win);
        win.focus();
        raise(win);
    }

    /** Focuses the most recently used shown window of a workspace other than `except`. */
    function focusNext(ws, except = null) {
        const next = byRecent(ws).find((win) => win !== except && win.visible);
        if (next !== undefined) {
            next.focus();
            raise(next);
        }
    }

    function hide(win) {
        const s = stateOf(win);
        s.minimized = true;
        win.setVisible(false);
        save(win);
    }

    function minimize(win) {
        closeUiOn(win.workspace);
        if (!win.visible) return;
        hide(win);
        const ws = win.workspace;
        if (ws.focused === win) focusNext(ws, win);
    }

    function unminimize(win) {
        const s = stateOf(win);
        s.minimized = false;
        win.setVisible(true);
        save(win);
    }

    /**
     * Minimizes every shown window of a workspace, or with none shown brings back the ones it
     * hid, with the focus where it was.
     */
    function showDesktop(ws) {
        const shown = ws.windows.filter((win) => win.visible);
        const before = desktops.get(ws.id);
        if (shown.length > 0) {
            // Windows still hidden from the last time come back together with these.
            const still = (before?.ids ?? []).filter((id) => {
                const win = bento.window(id);
                return win !== null && win.workspace === ws && !win.visible;
            });
            desktops.set(ws.id, {
                ids: [...still, ...shown.map((win) => win.id)],
                focused: ws.focused?.id ?? before?.focused ?? null,
            });
            shown.forEach(hide);
        } else if (before !== undefined) {
            desktops.delete(ws.id);
            // In the order they're stacked in, so they come back stacked the same.
            const ids = new Set(before.ids);
            const back = ws.windows.filter((win) => ids.has(win.id));
            back.forEach(unminimize);
            const focused = back.find((win) => win.id === before.focused) ?? back.at(-1);
            if (focused !== undefined) activate(focused);
        }
        bento.stateChanged();
    }

    // ---------------------------------------------------------------------------------------------
    // Snapping
    // ---------------------------------------------------------------------------------------------

    function snapTo(win, zone) {
        const s = stateOf(win);
        s.zone = { ...zone };
        if (win.mode !== "normal") win.setMode("normal");
        if (s.minimized || !win.visible) unminimize(win);
        apply(win);
        save(win);
    }

    /** Takes a window out of its zone and maximized mode, back to its own size and place. */
    function restore(win) {
        const s = stateOf(win);
        s.zone = null;
        if (win.mode !== "normal") win.setMode("normal");
        if (s.free === null) s.free = cascade(win.workspace, win.size, win);
        apply(win);
        save(win);
    }

    function maximize(win) {
        win.setMode("maximized");
        apply(win);
        save(win);
    }

    /**
     * Moves a window to the active workspace of the monitor on `side`, at the same place there: in
     * the same zone seen from that side, or at the same spot. Returns false without one.
     */
    function moveToMonitor(win, side, { mirrored = false } = {}) {
        const from = win.workspace;
        const monitor = from.monitor[side];
        if (monitor === null) return false;
        const to = monitor.activeWorkspace;
        track(to);
        const s = stateOf(win);
        if (s.zone !== null && mirrored) s.zone = mirror(s.zone);
        if (s.free !== null) s.free = mapRect(s.free, from, to);
        win.moveTo(to);
        apply(win);
        save(win);
        win.focus();
        raise(win);
        monitor.focus();
        focusNext(from);
        return true;
    }

    /**
     * `snap-left` and `snap-right`: a free or maximized window snaps to that half; from the
     * other side it goes back to its own size, and from that side it goes on to the monitor
     * there. A quarter goes to the quarter beside it.
     */
    function snapSide(win, side) {
        const s = stateOf(win);
        const toward = side === "left" ? LEFT : RIGHT;
        if (win.mode === "maximized" || s.zone === null) {
            snapTo(win, toward);
            offerAssist(win);
            return;
        }
        const z = s.zone;
        const atLeft = near(z.x, 0);
        const atRight = near(z.x + z.w, 1);
        const atNear = side === "left" ? atLeft : atRight;
        const atFar = side === "left" ? atRight : atLeft;
        if (atNear && !atFar) {
            moveToMonitor(win, side, { mirrored: true });
        } else if (atFar && !atNear) {
            if (near(z.w, 0.5) && z.h < 1 - EPSILON) {
                snapTo(win, mirror(z));
                offerAssist(win);
            } else {
                restore(win);
            }
        } else {
            snapTo(win, toward);
            offerAssist(win);
        }
    }

    /**
     * `snap-up`: a column goes to its top part, a bottom part back to the column, and anything
     * else is maximized.
     */
    function snapUp(win) {
        const z = stateOf(win).zone;
        if (win.mode === "maximized") return;
        if (z === null) {
            maximize(win);
        } else if (near(z.y, 0) && near(z.h, 1)) {
            snapTo(win, { ...z, h: 0.5 });
            offerAssist(win);
        } else if (z.y > EPSILON && near(z.y + z.h, 1)) {
            snapTo(win, { ...z, y: 0, h: 1 });
        } else {
            maximize(win);
        }
    }

    /**
     * `snap-down`: a maximized window is restored, a column goes to its bottom part, a top part
     * back to the column, and anything else is minimized.
     */
    function snapDown(win) {
        const z = stateOf(win).zone;
        if (win.mode === "maximized") {
            win.setMode("normal");
            apply(win);
            save(win);
        } else if (z !== null && near(z.y, 0) && near(z.h, 1)) {
            snapTo(win, { ...z, y: 0.5, h: 0.5 });
            offerAssist(win);
        } else if (z !== null && near(z.y, 0) && z.h < 1 - EPSILON) {
            snapTo(win, { ...z, y: 0, h: 1 });
        } else {
            minimize(win);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Resizing across shared edges
    // ---------------------------------------------------------------------------------------------

    /** The zones of a workspace's snapped windows, by window id. */
    function snappedZones(ws) {
        return new Map(
            ws.windows.filter(isSnapped).map((win) => [win.id, { ...stateOf(win).zone }]),
        );
    }

    /**
     * The windows along the edge of window `id` on `side` ("start" for its left or top edge) at
     * `line`, on `axis`: those with an edge on the line that overlap one already found across it,
     * each with the side of it on the line. An edge with windows on one side only is the window's
     * own.
     */
    function lineMembers(zones, id, side, axis, line) {
        const size = axis === "x" ? "w" : "h";
        const across = axis === "x" ? "y" : "x";
        const acrossSize = axis === "x" ? "h" : "w";
        const onLine = (z) =>
            near(z[axis], line) ? "start" : near(z[axis] + z[size], line) ? "end" : null;
        const overlap = (a, b) =>
            a[across] < b[across] + b[acrossSize] - EPSILON &&
            b[across] < a[across] + a[acrossSize] - EPSILON;
        const members = new Map([[id, side]]);
        for (let grew = true; grew; ) {
            grew = false;
            for (const [other, z] of zones) {
                const at = members.has(other) ? null : onLine(z);
                if (at !== null && [...members.keys()].some((m) => overlap(z, zones.get(m)))) {
                    members.set(other, at);
                    grew = true;
                }
            }
        }
        const shared = [...members.values()].some((at) => at !== side);
        return shared ? members : new Map([[id, side]]);
    }

    function resizeSnapped(win, edges, delta) {
        const ws = win.workspace;
        const span = snapSpan(ws);
        const start = resizing.zones;
        const zones = new Map([...start].map(([id, z]) => [id, { ...z }]));
        const own = start.get(win.id);
        for (const [axis, size, low, high, d] of [
            ["x", "w", "left", "right", delta.x / span.w],
            ["y", "h", "top", "bottom", delta.y / span.h],
        ]) {
            const min = MIN_SNAPPED / (axis === "x" ? span.w : span.h);
            for (const edge of [low, high]) {
                if (!edges[edge]) continue;
                const side = edge === low ? "start" : "end";
                const line = side === "start" ? own[axis] : own[axis] + own[size];
                const members = lineMembers(start, win.id, side, axis, line);
                // Every window along the line keeps at least the smallest size.
                let lo = 0;
                let hi = 1;
                for (const [id, at] of members) {
                    const z = start.get(id);
                    if (at === "start") hi = Math.min(hi, z[axis] + z[size] - min);
                    else lo = Math.max(lo, z[axis] + min);
                }
                const moved = clamp(line + d, lo, Math.max(lo, hi));
                for (const [id, at] of members) {
                    const from = start.get(id);
                    const z = zones.get(id);
                    if (at === "start") {
                        z[axis] = moved;
                        z[size] = from[axis] + from[size] - moved;
                    } else {
                        z[size] = moved - from[axis];
                    }
                }
            }
        }
        for (const [id, zone] of zones) {
            const other = bento.window(id);
            if (other === null) continue;
            stateOf(other).zone = zone;
            apply(other, { animate: false });
        }
    }

    function resizeFree(win, edges, delta) {
        const r = resizing.rect;
        const min = { w: Math.max(win.minSize.w, 48), h: Math.max(win.minSize.h, 48) };
        const max = { w: win.maxSize.w || Infinity, h: win.maxSize.h || Infinity };
        let { x, y, w, h } = r;
        if (edges.left || edges.right) {
            w = clamp(r.w + (edges.left ? -delta.x : delta.x), min.w, max.w);
            if (edges.left) x = r.x + r.w - w;
        }
        if (edges.top || edges.bottom) {
            h = clamp(r.h + (edges.top ? -delta.y : delta.y), min.h, max.h);
            if (edges.top) y = r.y + r.h - h;
        }
        const m = win.workspace.monitor;
        stateOf(win).free = { x: m.round(x), y: m.round(y), w: m.round(w), h: m.round(h) };
        apply(win, { animate: false });
    }

    // ---------------------------------------------------------------------------------------------
    // What's shown with the keyboard: the switcher, Snap Assist and the layouts picker
    // ---------------------------------------------------------------------------------------------
    //
    // One at a time, in `ui`: `{ kind, ws, count, selected, select(i), pick(i), teardown(picked) }`
    // and, for the switcher and Snap Assist, `windows`.

    /** Closes what's shown, and gives the keyboard back unless `release` is false. */
    function closeUi(picked = null, { release = true } = {}) {
        const current = ui;
        if (current === null) return;
        ui = null;
        current.teardown(picked);
        if (release && bento.keyboardGrabbed) bento.releaseKeyboard();
        bento.stateChanged();
    }

    /** Fades a node out, then takes it away. */
    function fadeAway(node) {
        node.set({ opacity: 0 });
        bento.setTimeout(() => node.remove(), MOTION.duration);
    }

    /** The scale and offset that fit `rect` in `box`, centred, never larger than it is. */
    function fit(rect, box) {
        const scale = Math.min(box.w / Math.max(1, rect.w), box.h / Math.max(1, rect.h), 1);
        return {
            scale,
            x: box.x + box.w / 2 - (rect.x + rect.w / 2),
            y: box.y + box.h / 2 - (rect.y + rect.h / 2),
        };
    }

    /** The whole view of a workspace, where a click outside what's shown closes it. */
    function viewRect(ws) {
        return { x: 0, y: 0, ...ws.monitor.size };
    }

    /**
     * Shows windows as thumbnails, each in a card with its title, over a panel, with a highlight
     * on the selected one. `cards` has `{ card, title, thumb }` rects for each window. Clicking a
     * card picks it, and pointing at one selects it; clicking elsewhere calls `cancel`.
     *
     * With `copies`, a thumbnail is a live copy of its window, and the windows stay where they
     * are. Without, it's the window itself, which leaves its place meanwhile and can then grow
     * from its thumbnail into where it goes.
     */
    function showThumbnails(ws, windows, cards, panel, selected, { kind, pick, copies = false }) {
        const order = ws.windows.slice();
        const temporary = new Set();
        const root = bento.createNode("box", {
            opacity: 1,
            transitions: { opacity: MOTION },
            enter: { opacity: 0 },
        });
        layerOf(ws).append(root);
        const catcher = bento.createNode("box", { rect: viewRect(ws), interactive: true });
        catcher.addEventListener("click", () => closeUi());
        root.append(catcher);
        root.append(
            bento.createNode("box", {
                rect: panel,
                cornerRadius: RADIUS + 4,
                background: PANEL,
                border: { width: 1, color: PANEL_EDGE },
                interactive: true,
            }),
        );
        const highlight = bento.createNode("box", {
            rect: grow(cards[selected].card, 3),
            cornerRadius: RADIUS,
            background: HIGHLIGHT,
            border: { width: 2, color: HIGHLIGHT_EDGE },
            transitions: { rect: MOTION },
        });
        root.append(highlight);

        const state = {
            kind,
            ws,
            windows,
            count: windows.length,
            selected,
            select(i) {
                state.selected = i;
                highlight.set({ rect: grow(cards[i].card, 3) });
                bento.stateChanged();
            },
            pick,
            teardown(picked) {
                if (copies) {
                    fadeAway(root);
                    return;
                }
                for (const win of windows) {
                    if (!ws.windows.includes(win)) continue;
                    win.node.set({ transform: IDENTITY, input: "app" });
                    if (temporary.has(win.id) && win !== picked) win.setVisible(false);
                }
                // Back where they were stacked.
                for (const win of order) {
                    if (ws.windows.includes(win)) ws.tree.append(win.node);
                }
                raiseLayer(ws);
                fadeAway(root);
            },
        };

        windows.forEach((win, i) => {
            const c = cards[i];
            const card = bento.createNode("box", {
                rect: c.card,
                cornerRadius: RADIUS - 2,
                interactive: true,
            });
            card.addEventListener("click", () => state.pick(i));
            card.addEventListener("pointerenter", () => state.select(i));
            root.append(card);
            card.append(
                bento.createNode("text", {
                    rect: c.title,
                    text: win.title ?? win.appId ?? "",
                    font: FONT,
                    color: TEXT,
                }),
            );
            if (copies) {
                // A copy shows a minimized window too, from what it last drew.
                card.append(
                    bento.createNode("copy", {
                        window: win,
                        rect: c.thumb,
                        cornerRadius: THUMB_RADIUS,
                    }),
                );
                return;
            }
            // A minimized window shows while it's a thumbnail.
            if (!win.visible) {
                temporary.add(win.id);
                win.setVisible(true);
            }
            win.node.set({ transitions: WINDOW_TRANSITIONS, input: "engine" });
            card.append(win.node);
            win.node.set({ transform: fit(win.rect ?? c.thumb, c.thumb) });
        });
        return state;
    }

    /** Cards for the switcher, in rows centred in the view, and the panel around them. */
    function switcherLayout(ws, windows) {
        const view = ws.monitor.workingArea;
        const m = ws.monitor;
        let height = clamp(Math.round(view.h * 0.2), 72, 200);
        for (let attempt = 0; ; attempt++) {
            const widths = windows.map((win) => {
                const r = win.rect ?? { w: 16, h: 10 };
                const w = (height * r.w) / Math.max(1, r.h);
                return Math.round(clamp(w, height * 0.6, height * 1.8));
            });
            const cardH = PAD + TITLE + PAD + height + PAD;
            const maxRow = view.w * 0.9 - 2 * OUTER;
            const rows = [[]];
            let rowW = 0;
            widths.forEach((w, i) => {
                const cardW = w + 2 * PAD;
                if (rows.at(-1).length > 0 && rowW + CARD_GAP + cardW > maxRow) {
                    rows.push([]);
                    rowW = 0;
                }
                rowW += (rows.at(-1).length > 0 ? CARD_GAP : 0) + cardW;
                rows.at(-1).push(i);
            });
            const rowWidth = (row) =>
                row.reduce((sum, i) => sum + widths[i] + 2 * PAD, 0) + (row.length - 1) * CARD_GAP;
            const innerH = rows.length * cardH + (rows.length - 1) * CARD_GAP;
            if (innerH + 2 * OUTER > view.h * 0.9 && attempt < 6) {
                height = Math.round(height * 0.75);
                continue;
            }
            const innerW = Math.max(...rows.map(rowWidth));
            const panel = {
                x: m.round(view.x + (view.w - innerW) / 2 - OUTER),
                y: m.round(view.y + (view.h - innerH) / 2 - OUTER),
                w: innerW + 2 * OUTER,
                h: innerH + 2 * OUTER,
            };
            const cards = [];
            rows.forEach((row, r) => {
                let x = panel.x + OUTER + (innerW - rowWidth(row)) / 2;
                const y = panel.y + OUTER + r * (cardH + CARD_GAP);
                for (const i of row) {
                    cards[i] = cardAt(m, x, y, widths[i], height);
                    x += widths[i] + 2 * PAD + CARD_GAP;
                }
            });
            return { cards, panel };
        }
    }

    /** A card at `x`, `y` with its title on top and a thumbnail `w` by `h` below. */
    function cardAt(m, x, y, w, h) {
        x = m.round(x);
        y = m.round(y);
        return {
            card: { x, y, w: w + 2 * PAD, h: PAD + TITLE + PAD + h + PAD },
            title: { x: x + PAD, y: y + PAD, w, h: TITLE },
            thumb: { x: x + PAD, y: y + PAD + TITLE + PAD, w, h },
        };
    }

    /** Cards for Snap Assist, in a grid that fills `rect` as well as it can. */
    function assistLayout(ws, windows, rect) {
        const m = ws.monitor;
        const n = windows.length;
        const inner = grow(rect, -OUTER);
        let best = null;
        for (let cols = 1; cols <= n; cols++) {
            const rows = Math.ceil(n / cols);
            const cellW = Math.min(420, (inner.w - (cols - 1) * CARD_GAP) / cols);
            const cellH = Math.min(320, (inner.h - (rows - 1) * CARD_GAP) / rows);
            const thumbW = cellW - 2 * PAD;
            const thumbH = cellH - TITLE - 3 * PAD;
            // Thumbnails are about as wide as windows: the more of that shape fits, the better.
            const score = Math.min(thumbW / 16, thumbH / 10);
            if (best === null || score > best.score) {
                best = { cols, rows, thumbW, thumbH, cellW, cellH, score };
            }
        }
        const { cols, rows, thumbW, thumbH, cellW, cellH } = best;
        const gridW = cols * cellW + (cols - 1) * CARD_GAP;
        const gridH = rows * cellH + (rows - 1) * CARD_GAP;
        const x0 = inner.x + (inner.w - gridW) / 2;
        const y0 = inner.y + (inner.h - gridH) / 2;
        const size = (v) => Math.max(1, Math.floor(v));
        const cards = windows.map((_, i) => {
            const col = i % cols;
            const row = Math.floor(i / cols);
            const x = x0 + col * (cellW + CARD_GAP);
            const y = y0 + row * (cellH + CARD_GAP);
            return cardAt(m, x, y, size(thumbW), size(thumbH));
        });
        return { cards, panel: rect };
    }

    /** Opens the window switcher on the focused monitor's workspace. */
    function openSwitcher(step) {
        const ws = bento.focusedMonitor?.activeWorkspace;
        if (ws === null || ws === undefined) return;
        if (ui !== null && ui.kind === "switcher" && ui.ws === ws) {
            ui.select((ui.selected + step + ui.count) % ui.count);
            return;
        }
        closeUi();
        const windows = byRecent(ws);
        if (windows.length === 0) return;
        // The keys held to open it: letting go of them picks. Shift goes back, so it doesn't count.
        const mods = bento.modifiers;
        const held = ["alt", "super", "ctrl"].filter((name) => mods[name]);
        const { cards, panel } = switcherLayout(ws, windows);
        const first = step > 0 ? Math.min(1, windows.length - 1) : windows.length - 1;
        ui = showThumbnails(ws, windows, cards, panel, first, {
            kind: "switcher",
            // The windows stay in their places behind the switcher.
            copies: true,
            pick: (i) => {
                const win = windows[i];
                closeUi(win);
                if (ws.windows.includes(win)) activate(win);
            },
        });
        ui.held = held;
        bento.grabKeyboard();
        bento.stateChanged();
    }

    /** Windows that Snap Assist can offer next to `win`: the others that aren't snapped. */
    function assistCandidates(win) {
        return byRecent(win.workspace).filter(
            (other) => other !== win && !isSnapped(other) && other.mode !== "fullscreen",
        );
    }

    /**
     * Offers the other windows to fill the space next to a window that just snapped: the first of
     * `zones`, or the largest free part of the working area. The rest of `zones` are offered after.
     * Returns whether it shows.
     */
    function offerAssist(win, zones = null) {
        const ws = win.workspace;
        if (!ws.options["snap-assist"]) return false;
        const candidates = assistCandidates(win);
        const zone = zones?.[0] ?? freeZone(ws);
        if (candidates.length === 0 || zone === null) return false;
        closeUi(null, { release: false });
        const rect = zoneRect(ws, zone);
        const { cards, panel } = assistLayout(ws, candidates, rect);
        const rest = zones?.slice(1) ?? [];
        ui = showThumbnails(ws, candidates, cards, panel, 0, {
            kind: "assist",
            pick: (i) => {
                const picked = candidates[i];
                closeUi(picked, { release: false });
                if (!ws.windows.includes(picked)) return;
                snapTo(picked, zone);
                picked.focus();
                raise(picked);
                if (!offerAssist(picked, rest.length > 0 ? rest : null) && bento.keyboardGrabbed) {
                    bento.releaseKeyboard();
                }
            },
        });
        ui.zone = zone;
        // After a drop the keyboard can't be taken, and Snap Assist works with the pointer only.
        bento.grabKeyboard();
        bento.stateChanged();
        return true;
    }

    /** Shows the layouts picker near the top of the view, for `win`. */
    function openLayouts(win) {
        const ws = win.workspace;
        closeUi();
        const view = ws.monitor.workingArea;
        const m = ws.monitor;
        const frameW = 112;
        const frameH = Math.round((frameW * view.h) / Math.max(1, view.w));
        const panelW = LAYOUTS.length * frameW + (LAYOUTS.length - 1) * CARD_GAP + 2 * PAD;
        const panel = {
            x: m.round(view.x + (view.w - panelW) / 2),
            y: view.y + OUTER,
            w: panelW,
            h: frameH + 2 * PAD,
        };
        const root = bento.createNode("box", {
            opacity: 1,
            transitions: { opacity: MOTION },
            enter: { opacity: 0 },
        });
        layerOf(ws).append(root);
        const catcher = bento.createNode("box", { rect: viewRect(ws), interactive: true });
        catcher.addEventListener("click", () => closeUi());
        root.append(catcher);
        root.append(
            bento.createNode("box", {
                rect: panel,
                cornerRadius: RADIUS,
                background: PANEL,
                border: { width: 1, color: PANEL_EDGE },
                interactive: true,
            }),
        );

        const zones = [];
        const nodes = [];
        LAYOUTS.forEach((layout, l) => {
            const frame = {
                x: panel.x + PAD + l * (frameW + CARD_GAP),
                y: panel.y + PAD,
                w: frameW,
                h: frameH,
            };
            for (const zone of layout) {
                const i = zones.length;
                zones.push({ layout, zone });
                const rect = {
                    x: m.round(frame.x + zone.x * frame.w + 2),
                    y: m.round(frame.y + zone.y * frame.h + 2),
                    w: Math.max(1, m.round(zone.w * frame.w - 4)),
                    h: Math.max(1, m.round(zone.h * frame.h - 4)),
                };
                const node = bento.createNode("box", {
                    rect,
                    cornerRadius: 4,
                    background: i === 0 ? ZONE_PICKED : ZONE,
                    transitions: { background: MOTION },
                    interactive: true,
                });
                node.addEventListener("click", () => state.pick(i));
                node.addEventListener("pointerenter", () => state.select(i));
                root.append(node);
                nodes.push(node);
            }
        });

        const state = {
            kind: "layouts",
            ws,
            window: win,
            count: zones.length,
            selected: 0,
            select(i) {
                nodes[state.selected].set({ background: ZONE });
                state.selected = i;
                nodes[i].set({ background: ZONE_PICKED });
                bento.stateChanged();
            },
            pick(i) {
                const { layout, zone } = zones[i];
                closeUi(null, { release: false });
                if (!ws.windows.includes(win)) return;
                snapTo(win, zone);
                win.focus();
                raise(win);
                const rest = layout.filter((other) => other !== zone);
                if (!offerAssist(win, rest) && bento.keyboardGrabbed) bento.releaseKeyboard();
            },
            teardown() {
                fadeAway(root);
            },
        };
        ui = state;
        bento.grabKeyboard();
        bento.stateChanged();
    }

    bento.addEventListener("keydown", (e) => {
        if (ui === null) return;
        const steps = { Right: 1, Down: 1, Left: -1, Up: -1, ISO_Left_Tab: -1 };
        const step = e.key === "Tab" ? (e.modifiers.shift ? -1 : 1) : steps[e.key];
        if (step !== undefined) {
            ui.select((ui.selected + step + ui.count) % ui.count);
        } else if (e.key === "Return" || e.key === "KP_Enter" || e.key === "space") {
            ui.pick(ui.selected);
        } else if (e.key === "Escape") {
            e.preventDefault();
            closeUi();
        }
    });

    bento.addEventListener("keyboardrelease", (e) => {
        // The engine's own releases are already done with.
        if (e.reason !== "engine") closeUi(null, { release: false });
    });

    bento.addEventListener("modifierschange", (e) => {
        if (ui?.kind === "switcher" && ui.held.length > 0) {
            if (!ui.held.some((name) => e.modifiers[name])) ui.pick(ui.selected);
        }
    });

    // ---------------------------------------------------------------------------------------------
    // Windows
    // ---------------------------------------------------------------------------------------------

    /** Closes what's shown on a workspace, when windows come and go there. */
    function closeUiOn(ws) {
        if (ui !== null && ui.ws === ws) closeUi();
    }

    function onRemove(e) {
        const win = e.target;
        const ws = win.workspace;
        closeUiOn(ws);
        if (resizing?.id === win.id) resizing = null;
        // A dragged window comes back when it's dropped, with what it had.
        if (e.reason === "drag") return;
        forget(win);
        if (ws.focused === null || ws.focused === win) focusNext(ws, win);
        bento.stateChanged();
    }

    /** bento moved a window to another workspace, with one of its own actions. */
    function onMove(e) {
        const win = e.target;
        closeUiOn(e.from);
        closeUiOn(e.to);
        track(e.to);
        const s = stateOf(win);
        if (s.free !== null) s.free = mapRect(s.free, e.from, e.to);
        apply(win);
        save(win);
        if (e.activate) {
            win.focus();
            raise(win);
        } else {
            raiseLayer(e.to);
        }
        if (e.from.focused === null) focusNext(e.from);
    }

    /** Maximized fills the working area and fullscreen the monitor; leaving goes back. */
    function onModeRequest(e) {
        e.preventDefault();
        const win = e.target;
        win.setMode(e.mode);
        apply(win);
        save(win);
        if (win.workspace.focused === win) raise(win);
    }

    /** A click, an app or a taskbar asks for a window: it comes back if it's minimized. */
    function onActivateRequest(e) {
        const win = e.target;
        if (ui !== null && !ui.windows?.includes(win)) closeUi();
        activate(win);
    }

    function onMinimizeRequest(e) {
        minimize(e.target);
    }

    /** With floating off, dialogs come here: they're centred over their parent. */
    function onFloatRequest(e) {
        const win = e.target;
        if (e.reason !== "dialog" || e.parent === null || e.parent.workspace !== win.workspace) {
            // Every window is free already.
            return;
        }
        const s = stateOf(win);
        // The size it asked for is without the borders.
        const size =
            e.size === null
                ? win.size
                : { w: e.size.w + win.borders.w, h: e.size.h + win.borders.h };
        s.free = centredOver(e.parent, size);
        s.zone = null;
        if (win.mode !== "normal") win.setMode("normal");
        apply(win);
        save(win);
    }

    function onResizeBegin(e) {
        const win = e.target;
        if (win.mode !== "normal" || win.rect === null) {
            e.preventDefault();
            return;
        }
        closeUiOn(win.workspace);
        const snapped = stateOf(win).zone !== null;
        resizing = {
            id: win.id,
            rect: { ...win.rect },
            zones: snapped ? snappedZones(win.workspace) : null,
        };
    }

    function onResizeUpdate(e) {
        const win = e.target;
        if (resizing?.id !== win.id) return;
        if (resizing.zones !== null) resizeSnapped(win, e.edges, e.delta);
        else resizeFree(win, e.edges, e.delta);
    }

    function onResizeEnd(e) {
        if (resizing?.id !== e.target.id) return;
        const ids = resizing.zones === null ? [e.target.id] : [...resizing.zones.keys()];
        resizing = null;
        for (const id of ids) {
            const win = bento.window(id);
            if (win !== null) save(win);
        }
    }

    function listen(win) {
        win.addEventListener("remove", onRemove);
        win.addEventListener("move", onMove);
        win.addEventListener("moderequest", onModeRequest);
        win.addEventListener("activaterequest", onActivateRequest);
        win.addEventListener("minimizerequest", onMinimizeRequest);
        win.addEventListener("floatrequest", onFloatRequest);
        win.addEventListener("resizebegin", onResizeBegin);
        win.addEventListener("resizeupdate", onResizeUpdate);
        win.addEventListener("resizeend", onResizeEnd);
    }

    /**
     * A window dropped at the end of a drag: snapped where it was dropped, or free there. A window
     * that was snapped or maximized gets its own size back, under the pointer where it held it.
     */
    function dropped(win, e) {
        const ws = win.workspace;
        const s = stateOf(win);
        hidePreview(ws);
        const wasFree = isFree(win);
        const size = fitSize(ws, s.free ?? e.rect ?? defaultSize(ws));
        const target = e.pointer === null ? null : dropTarget(ws, e.pointer);
        if (win.mode !== "normal") win.setMode("normal");
        s.minimized = false;
        // Snapping by the drop keeps the place it had before as its own.
        if (target === null || s.free === null) {
            let free;
            if (e.rect === null) {
                free = cascade(ws, size, win);
            } else if (wasFree) {
                free = { x: e.rect.x, y: e.rect.y, ...size };
            } else {
                const p = e.pointer ?? { x: e.rect.x + e.rect.w / 2 };
                const held = (p.x - e.rect.x) / Math.max(1, e.rect.w);
                free = { x: p.x - held * size.w, y: e.rect.y, ...size };
            }
            s.free = reachable(ws, free);
        }
        s.zone = target === null || target === MAXIMIZE ? null : { ...target };
        if (target === MAXIMIZE) win.setMode("maximized");
        apply(win);
        save(win);
        if (e.activate) {
            win.focus();
            raise(win);
        }
        if (s.zone !== null) offerAssist(win);
    }

    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        const ws = win.workspace;
        track(ws);
        listen(win);
        closeUiOn(ws);
        if (!recent.includes(win.id)) recent.push(win.id);

        if (e.reason === "drop") {
            dropped(win, e);
            return;
        }

        const s = stateOf(win);
        if (e.reason === "adopt" && s.free !== null) {
            // It was this engine's before.
        } else if (e.reason !== "open" && e.rect !== null) {
            const shift = cameraShift.get(ws.id) ?? { x: 0, y: 0 };
            const r = e.rect;
            s.free = reachable(ws, { ...fitSize(ws, r), x: r.x - shift.x, y: r.y - shift.y });
        } else {
            const asked = win.size.w > 0 && win.size.h > 0 ? win.size : defaultSize(ws);
            const parent = e.nextTo !== null && e.nextTo.workspace === ws ? e.nextTo : null;
            s.free = parent !== null ? centredOver(parent, asked) : cascade(ws, asked, win);
        }
        win.setVisible(!s.minimized);
        apply(win);
        save(win);
        if (e.activate && !s.minimized) {
            win.focus();
            raise(win);
        } else {
            raiseLayer(ws);
        }
    });

    // Windows open at the size they ask for: the window decides.
    bento.addEventListener("newwindowsize", (e) => e.respondWith({ w: 0, h: 0 }));

    bento.addEventListener("focuschange", () => {
        const win = bento.focused;
        if (win === null) return;
        touch(win);
        raise(win);
    });

    bento.addEventListener("action", (e) => {
        const switching = e.name === "switch-windows" || e.name === "switch-windows-back";
        if (!(switching && ui?.kind === "switcher")) closeUi();
        const win = e.window;
        if (switching) {
            openSwitcher(e.name === "switch-windows" ? 1 : -1);
            return;
        }
        if (e.name === "show-desktop") {
            const ws = win?.workspace ?? bento.focusedMonitor?.activeWorkspace;
            if (ws) showDesktop(ws);
            return;
        }
        // The rest act on a window that shows.
        if (win === null || !win.visible || win.mode === "fullscreen") return;
        switch (e.name) {
            case "snap-left":
                return snapSide(win, "left");
            case "snap-right":
                return snapSide(win, "right");
            case "snap-up":
                return snapUp(win);
            case "snap-down":
                return snapDown(win);
            case "show-layouts":
                return openLayouts(win);
            case "minimize":
                return minimize(win);
            case "move-to-monitor-left":
                return moveToMonitor(win, "left");
            case "move-to-monitor-right":
                return moveToMonitor(win, "right");
        }
    });

    // Monitors and the config: snapped and maximized windows follow the working area and gaps.
    function placeAll() {
        for (const win of bento.windows) apply(win);
    }
    bento.addEventListener("configchange", placeAll);
    const watchMonitor = (monitor) => monitor.addEventListener("change", placeAll);
    bento.monitors.forEach(watchMonitor);
    bento.addEventListener("monitoradd", (e) => watchMonitor(e.monitor));
    bento.addEventListener("workspaceadd", (e) => track(e.workspace));
    for (const monitor of bento.monitors) monitor.workspaces.forEach(track);

    bento.addEventListener("unload", (e) => {
        e.saveState({ recent, desktops: [...desktops] });
    });

    // Readable from outside: where each window is snapped, which are minimized, the order Alt+Tab
    // goes in, and what's shown.
    bento.exposeState(() => ({
        windows: bento.windows.map((win) => {
            const s = stateOf(win);
            return { id: win.id, zone: s.zone, free: s.free, minimized: s.minimized };
        }),
        recent: recent.filter((id) => bento.window(id) !== null),
        showing:
            ui === null
                ? null
                : {
                      kind: ui.kind,
                      workspace: ui.ws.id,
                      windows: ui.windows?.map((win) => win.id) ?? null,
                      selected: ui.selected,
                  },
    }));
}
