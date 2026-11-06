// A template for bento layout engines: main and stack.
//
// A layout engine runs the presentation of the whole session: where windows go, which one has
// focus, and anything it draws. This one puts one window, the main one, on the left part of each
// workspace's view, and the others share the rest, stacked top to bottom, with a line between
// them. It's a complete engine, small enough to read in one go: copy it, give it a name in `meta`,
// and change what it does. Load it from the config:
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
// bento runs the engine once for the whole session, and tells it what happens by events, the way
// the DOM does. The engine keeps a model of the windows (here, an array for each workspace),
// changes the model when something happens, then calls `layout()`, which places every window from
// the model. Placing a window where it already is does nothing, so `layout()` doesn't need to know
// what changed.

export const meta = {
    api: 2,
    name: "main-and-stack",
    description: "One main window on the left, the others stacked on the right.",
    // Every action the engine has, bound by name in the engine's .kdl file, like
    // `Mod+P { promote; }`. Settings screens list these, and bento checks binds against them.
    actions: {
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

const clamp = (value, min, max) => Math.min(Math.max(value, min), max);

export default function create(bento) {
    // The model: each workspace's windows in order, the main one first, by workspace id.
    const orders = new Map();
    // What the engine draws: the line between the main window and the stack, by workspace id.
    const dividers = new Map();

    function orderOf(ws) {
        if (!orders.has(ws.id)) {
            orders.set(ws.id, []);
        }
        return orders.get(ws.id);
    }

    /** Places every window of a workspace from the model. */
    function layout(ws) {
        // The monitor's working area without panels, and the config's struts. The camera stays at
        // the canvas origin, so canvas coordinates are view coordinates.
        const area = ws.tilingArea;
        const gap = ws.config.gaps;
        const monitor = ws.monitor;
        const [main, ...stack] = orderOf(ws);
        const mainWidth = clamp(bento.options["main-width"], 0.1, 0.9);

        // The main window is as wide as the view when it's alone.
        const mainW =
            stack.length === 0 ? area.w - gap * 2 : monitor.round((area.w - gap * 3) * mainWidth);
        const rects = new Map();
        if (main) {
            rects.set(main, { x: area.x + gap, y: area.y + gap, w: mainW, h: area.h - gap * 2 });
        }

        // The stack shares the rest of the height evenly.
        const x = area.x + gap * 2 + mainW;
        const w = Math.max(1, area.x + area.w - gap - x);
        const h = (area.h - gap * (stack.length + 1)) / stack.length;
        stack.forEach((win, i) => {
            const top = monitor.round(area.y + gap + i * (h + gap));
            const bottom = monitor.round(area.y + gap + i * (h + gap) + h);
            rects.set(win, { x, y: top, w, h: Math.max(1, bottom - top) });
        });

        for (const win of orderOf(ws)) {
            // Fullscreen windows cover the whole monitor, and maximized ones its working area.
            const rect =
                win.mode === "fullscreen"
                    ? { x: 0, y: 0, ...monitor.size }
                    : win.mode === "maximized"
                      ? monitor.workingArea
                      : rects.get(win);
            win.place(rect);
        }

        // Draw the focused window above the others, which matters for fullscreen windows.
        ws.focused?.raise();
        drawDivider(
            ws,
            stack.length > 0 ? { x: x - gap / 2 - 1, y: area.y + gap, w: 2, h: area.h - gap * 2 } : null,
        );
    }

    /** Draws a line at `rect` on a workspace, or takes it away with `null`. */
    function drawDivider(ws, rect) {
        let node = dividers.get(ws.id);
        if (rect === null) {
            node?.remove();
        } else {
            if (node === undefined) {
                // Nodes are drawn once they're in a workspace's tree, above the windows already
                // there. `transitions` animate a change of `rect` like the windows move.
                node = bento.createNode("box", {
                    background: "#808080",
                    transitions: { rect: "window-movement" },
                });
                dividers.set(ws.id, node);
            }
            node.set({ rect });
            ws.tree.append(node);
        }
    }

    // Events from windows. They share one function each, so a window that comes back to the
    // engine, like after floating, doesn't get its listeners twice.

    function onRemove(e) {
        const win = e.target;
        const ws = win.workspace;
        const order = orderOf(ws);
        const idx = order.indexOf(win);
        if (idx === -1) {
            return;
        }
        order.splice(idx, 1);
        // If the window had focus, bento may have taken it away already: give it to the one that
        // takes its place, or the one before if it was last.
        if ((ws.focused === null || ws.focused === win) && order.length > 0) {
            order[Math.min(idx, order.length - 1)].focus();
        }
        layout(ws);
    }

    /** bento moved a window to another workspace, with one of its own actions. */
    function onMove(e) {
        const win = e.target;
        const from = orderOf(e.from);
        const to = orderOf(e.to);
        if (!from.includes(win)) {
            return;
        }
        from.splice(from.indexOf(win), 1);
        to.splice(Math.min(1, to.length), 0, win);
        if (e.activate) {
            win.focus();
        }
        layout(e.from);
        layout(e.to);
    }

    /**
     * A window, or an action like `fullscreen-window`, asks for fullscreen, maximized or normal.
     * Without a listener, the window gets the mode but stays where it is.
     */
    function onModeRequest(e) {
        e.preventDefault();
        e.target.setMode(e.mode);
        layout(e.target.workspace);
    }

    /** Something wants focus on a window: a click, an app, or the socket. bento focuses it. */
    function onActivateRequest(e) {
        e.target.focus();
    }

    // Events from bento.

    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        const order = orderOf(win.workspace);
        const saved = bento.savedState?.order ?? [];
        if (e.reason === "adopt" && saved.includes(win.id)) {
            // Windows that were there when the engine started go back in their saved order.
            const rank = (other) => saved.indexOf(other.id);
            const idx = order.findIndex((other) => rank(other) > rank(win));
            order.splice(idx === -1 ? order.length : idx, 0, win);
        } else if (
            e.reason === "drop" &&
            e.pointer !== null &&
            e.pointer.x < win.workspace.monitor.size.w / 2
        ) {
            // Dropped on the left half of the view: it becomes the main window.
            order.unshift(win);
        } else if (e.nextTo !== null && order.includes(e.nextTo)) {
            // Next to the window it belongs with, like a dialog after its parent.
            order.splice(order.indexOf(e.nextTo) + 1, 0, win);
        } else {
            // On top of the stack.
            order.splice(Math.min(1, order.length), 0, win);
        }

        win.addEventListener("remove", onRemove);
        win.addEventListener("move", onMove);
        win.addEventListener("moderequest", onModeRequest);
        win.addEventListener("activaterequest", onActivateRequest);

        if (e.activate) {
            win.focus();
        }
        layout(win.workspace);
    });

    // The one action. `e.window` is the focused window, or the one given on the command line.
    bento.addEventListener("action", (e) => {
        if (e.name !== "promote" || e.window === null) {
            return;
        }
        const ws = e.window.workspace;
        const order = orderOf(ws);
        const idx = order.indexOf(e.window);
        // The main window swaps with the top of the stack, any other window with the main one.
        const other = idx === 0 ? 1 : 0;
        if (idx !== -1 && other < order.length) {
            [order[idx], order[other]] = [order[other], order[idx]];
            layout(ws);
        }
    });

    // Monitors change size, and the config changes `main-width`: place everything again.
    function layoutAll() {
        for (const monitor of bento.monitors) {
            monitor.workspaces.forEach(layout);
        }
    }
    bento.addEventListener("configchange", layoutAll);
    for (const monitor of bento.monitors) {
        monitor.addEventListener("change", () => monitor.workspaces.forEach(layout));
    }
    bento.addEventListener("monitoradd", (e) => {
        e.monitor.addEventListener("change", () => e.monitor.workspaces.forEach(layout));
    });

    // The order goes with the engine when it reloads, by window id.
    bento.addEventListener("unload", (e) => {
        e.saveState({ order: [...orders.values()].flat().map((win) => win.id) });
    });

    // Left out here, with what bento does without them:
    // - the `workspaceadd` and `remove` events: the model is made when a workspace is first seen.
    // - `swipebegin`, `swipeupdate` and `swipeend` on a workspace: horizontal swipes do nothing.
    // - `resizebegin` on a window: resizing with the mouse is refused.
    // - `dragenter`, `dragmove` and `dragleave` on a workspace: no hint where a window would land.
    // - the `newwindowsize` query: windows pick their own size to open at.
    // - `windowRules` in `meta`: rules an engine declares, which windows get as `win.rules`.
    // Tabs are up to the engine too, with `bento.createGroup`; see `../bento.js` and `../niri.js`.
}
