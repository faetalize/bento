// A template for bento layout engines: main and stack.
//
// A layout engine decides how a whole workspace behaves, out of the small primitives that bento
// gives it. This one puts one window, the main one, on the left part of the view, and the others
// share the rest, stacked top to bottom. The view never scrolls. It's a complete engine, small
// enough to read in one go: copy it, give it a name in `meta`, and change what it does. Load it
// from the config:
//
//     tiling {
//         layout "~/path/to/my-engine.js"
//     }
//
// bento reloads the file when it changes. The engine's options and binds go in `my-engine.kdl` next
// to it, loaded only while it runs, with its actions bound by name. The full API is in
// `../bento-layout.d.ts`, and `template.test.js` next to this file shows how to test an engine
// with node, without bento.
//
// Most engines take the same shape as this one: keep a model of the windows (here, an array and a
// width), change the model when something happens, then call `apply()`, which places every window
// from the model. Placing a window where it already is does nothing, so `apply()` doesn't need to
// know what changed.

export const meta = {
    api: 1,
    name: "main-and-stack",
    description: "One main window on the left, the others stacked on the right.",
    // Every action the engine has, bound by name in the engine's .kdl file, like
    // `Mod+P { promote; }`. Settings screens list these, and bento checks binds against them.
    actions: {
        "focus-main": { description: "Focus the main window." },
        "focus-stack": { description: "Focus the stack." },
        "focus-up": { description: "Focus the window above in the stack." },
        "focus-down": { description: "Focus the window below in the stack." },
        "move-up": { description: "Move the focused window up the stack." },
        "move-down": { description: "Move the focused window down the stack." },
        "set-main-width": {
            description: "Set or change the main window's width.",
            args: [{ name: "change", type: "size" }],
        },
        promote: { description: "Swap the focused window with the main one." },
    },
    // Options for the `options {}` section of the engine's .kdl file.
    options: {
        "main-width": {
            type: "number",
            default: 0.55,
            description: "Width of the main window, as a proportion of the view.",
        },
    },
};

const MIN_WIDTH = 0.1;
const MAX_WIDTH = 0.9;

const clamp = (value, min, max) => Math.min(Math.max(value, min), max);

export default function create(ws) {
    // The model: the windows in order, the main one first, and the main window's width. It's all
    // that `apply()` needs, and all that `save()` keeps when the engine reloads.
    let order = [];
    let optionWidth = ws.options["main-width"];
    let mainWidth = clamp(optionWidth, MIN_WIDTH, MAX_WIDTH);

    /** Places every window from the model. */
    function apply() {
        // The view without panels and the config's struts. The camera stays at the canvas origin,
        // so canvas coordinates are view coordinates.
        const area = ws.view.tilingArea;
        const gap = ws.config.gaps;
        const [main, ...stack] = order;
        const rects = new Map();

        // The main window is as wide as the view when it's alone.
        const mainW =
            stack.length === 0 ? area.w - gap * 2 : ws.round((area.w - gap * 3) * mainWidth);
        if (main) {
            rects.set(main, { x: area.x + gap, y: area.y + gap, w: mainW, h: area.h - gap * 2 });
        }

        // The stack shares the rest of the height evenly.
        const x = area.x + gap * 2 + mainW;
        const w = Math.max(1, area.x + area.w - gap - x);
        const h = (area.h - gap * (stack.length + 1)) / stack.length;
        stack.forEach((win, i) => {
            const top = ws.round(area.y + gap + i * (h + gap));
            const bottom = ws.round(area.y + gap + i * (h + gap) + h);
            rects.set(win, { x, y: top, w, h: Math.max(1, bottom - top) });
        });

        for (const win of order) {
            // Fullscreen windows cover the whole view, and maximized ones the view without panels.
            // Windows that can't take their size get drawn at their own size, at the same place.
            const rect =
                win.mode === "fullscreen"
                    ? { x: 0, y: 0, ...ws.view.size }
                    : win.mode === "maximized"
                      ? ws.view.workingArea
                      : rects.get(win);
            ws.place(win, rect);
        }
        // Draw the focused window above the others, which matters for fullscreen windows.
        if (ws.focused !== null) {
            ws.raise(ws.focused);
        }
    }

    /** Focuses the window at `idx` in the order, if there is one. */
    function focusAt(idx) {
        if (idx < 0 || idx >= order.length) {
            return false;
        }
        ws.focus(order[idx]);
        apply();
        return true;
    }

    /** Swaps the windows at `a` and `b` in the order, if there are both. */
    function swap(a, b) {
        if (a === b || b < 0 || b >= order.length) {
            return false;
        }
        [order[a], order[b]] = [order[b], order[a]];
        apply();
        return true;
    }

    /** `set-main-width`: `"%"` is a percentage of the working area, like in the config. */
    function setMainWidth(change) {
        const full = ws.view.tilingArea.w;
        const value = change.unit === "%" ? change.value / 100 : change.value / full;
        const next = clamp(change.op === "set" ? value : mainWidth + value, MIN_WIDTH, MAX_WIDTH);
        if (next === mainWidth) {
            return false;
        }
        mainWidth = next;
        apply();
        return true;
    }

    return {
        // The two events every engine must handle.

        windowAdded(win, how) {
            const left = how.pointer !== null && how.pointer.x < ws.view.size.w / 2;
            if (how.reason === "drop" && left) {
                // Dropped on the left half of the view: it becomes the main window.
                order.unshift(win);
            } else if (how.nextTo !== null && order.includes(how.nextTo)) {
                // Next to the window it belongs with, like a dialog after its parent.
                order.splice(order.indexOf(how.nextTo) + 1, 0, win);
            } else {
                // On top of the stack.
                order.splice(Math.min(1, order.length), 0, win);
            }
            if (how.activate) {
                ws.focus(win);
            }
            apply();
        },

        windowRemoved(win) {
            const idx = order.indexOf(win);
            order.splice(idx, 1);
            // If the window had focus, bento took it away: give it to the window that took its
            // place, or the one before if it was last.
            if (ws.focused === null && order.length > 0) {
                ws.focus(order[Math.min(idx, order.length - 1)]);
            }
            apply();
        },

        // Optional events. Each comment says what bento does without them.

        /** Something wants focus on a window: a click, an app, or IPC. Default: focus it. */
        activate(win) {
            ws.focus(win);
            apply();
        },

        /**
         * A window, or an action like `fullscreen-window`, asks for fullscreen, maximized or
         * normal. Default: the window gets the mode, but stays where it is.
         */
        modeRequested(win, mode) {
            ws.setMode(win, mode);
            apply();
        },

        /**
         * The actions from `meta.actions`, by name, with their arguments. Returns whether
         * anything happened. Default: none do anything.
         */
        action(name, args) {
            const idx = order.indexOf(ws.focused);
            if (idx === -1) {
                return false;
            }
            switch (name) {
                // Swaps the main window with the top of the stack when it's the focused one.
                case "promote":
                    return swap(idx, idx === 0 ? 1 : 0);
                case "focus-main":
                    return idx > 0 && focusAt(0);
                case "focus-stack":
                    return idx === 0 && focusAt(1);
                case "focus-up":
                    return idx > 1 && focusAt(idx - 1);
                case "focus-down":
                    return idx > 0 && focusAt(idx + 1);
                case "move-up":
                    return idx > 1 && swap(idx, idx - 1);
                case "move-down":
                    return idx > 0 && swap(idx, idx + 1);
                case "set-main-width":
                    return setMainWidth(args[0]);
                default:
                    return false;
            }
        },

        /** The output size, working area or scale changed. Default: nothing moves. */
        viewChanged() {
            apply();
        },

        /** The config changed. Default: nothing moves. */
        configChanged() {
            // Take a new `main-width` from the config, but keep the width otherwise.
            if (ws.options["main-width"] !== optionWidth) {
                optionWidth = ws.options["main-width"];
                mainWidth = clamp(optionWidth, MIN_WIDTH, MAX_WIDTH);
            }
            apply();
        },

        /** A swipe gesture moved the view. Default: the view stays where the swipe left it. */
        gestureEnd() {
            ws.moveCamera({ x: 0, y: 0 });
        },

        /** State to keep when the engine reloads. Default: nothing. */
        save() {
            return { order: order.map((win) => win.id), mainWidth };
        },

        /**
         * The engine takes over windows: when it loads, reloads, or the config switches to it.
         * `state` is what `save()` returned, if this engine saved anything. Default: each window
         * gets `windowAdded` with reason `"adopt"`.
         */
        adopt(windows, focused, state) {
            // Saved windows go back in their order, and others by where they were, left to right.
            const saved = Array.isArray(state?.order) ? state.order : [];
            const rank = (win) => (saved.includes(win.id) ? saved.indexOf(win.id) : Infinity);
            order = [...windows].sort(
                (a, b) =>
                    rank(a) - rank(b) ||
                    (a.rect?.x ?? 0) - (b.rect?.x ?? 0) ||
                    (a.rect?.y ?? 0) - (b.rect?.y ?? 0),
            );
            if (Number.isFinite(state?.mainWidth)) {
                mainWidth = clamp(state.mainWidth, MIN_WIDTH, MAX_WIDTH);
            }
            ws.moveCamera({ x: 0, y: 0 }, { animate: false });
            ws.focus(focused ?? order[0] ?? null);
            apply();
        },

        // Left out here, with what bento does without them:
        // - windowChanged(win): a window's title, size limits or size changed. Nothing moves.
        // - gestureBegin(): a swipe starts. Nothing happens.
        // - resizeBegin, resizeUpdate, resizeEnd: resizing with the mouse. bento refuses it.
        // - dropTarget(win, pointer, rect): where a dragged window would land. No highlight.
        // - newWindowSize(info): the size a window opens at. The window picks it.
        // - ipcPosition(win): a window's column and row for IPC clients. None.
        // Tabs are up to the engine too, with `ws.setTabBar` and `ws.tabIndicatorSpace`; see
        // `../bento.js` and `../niri.js`.
    };
}
