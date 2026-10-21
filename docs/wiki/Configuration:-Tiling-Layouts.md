### Overview

In bento, tiling is done by a layout engine: a script that decides how the whole workspace behaves.
bento itself only knows about windows and a camera over an unbounded canvas, and gives engines small primitives to work with.
The engine decides everything else: where tiled windows go, which window has focus, where the camera points, and what actions do.
Concepts like columns, cells or gravity belong to the engine, not to bento.

bento ships two layout engines:

- `bento`, the default: windows on a grid of cells. See [Grid Layout](./Grid-Layout.md).
- `niri`: niri's scrollable columns, as in niri. See [The niri Engine](./Niri-Engine.md).

Pick one, or an engine file, in the `tiling` section:

```kdl
tiling {
    layout "niri"
}
```

```kdl
tiling {
    // A path, absolute or relative to the config file.
    layout "~/.config/bento/layouts/spiral.js"
}
```

Engines aren't interchangeable presets.
Each one defines how the workspace works, with its own actions and its own meaning for niri's actions, so switching engines usually means changing your binds too.

When the engine file changes, bento reloads it, just like the config.
Engines can keep their state across reloads, so your windows stay where they are while you work on an engine.

### Options

Engines declare their own options, set in `options`:

```kdl
tiling {
    layout "bento"
    options {
        gravity false
    }
}
```

Values are booleans, numbers or strings.
`bento validate` checks the options against what the engine declares, and so does bento when it loads the config.

The `layout {}` section of the config still applies too.
Engines get its values, such as `gaps`, `struts`, `preset-column-widths` or `center-focused-column`, and decide what they mean.
Borders, focus rings, shadows and the tab indicator look the way the `layout {}` section says, whatever the engine.

### Actions

niri's layout actions, like `focus-column-left`, `move-window-down` or `set-column-width`, go to the engine under their own names.
Each engine gives them its meaning: in the `niri` engine they work like in niri, and in the `bento` engine they move around the grid.

That includes actions that combine others, like `focus-column-or-monitor-left` or `focus-window-or-workspace-down`.
The engine decides what happens within the workspace, and when it does nothing, bento does the rest: it focuses the monitor to the left, or switches to the workspace below.

Engines can also add actions of their own.
Bind them with `layout-action`, followed by the action's name and any arguments:

```kdl
binds {
    Mod+Shift+S { layout-action "swap-with-first"; }
    Mod+Shift+R { layout-action "resize" 2 "right"; }
}
```

Run them from the command line the same way:

```sh
bento msg action layout-action swap-with-first
```

`bento validate` checks that the engine has the actions your binds name.

### Writing a layout engine

A layout engine is a single JavaScript module.
It exports `meta`, which describes the engine, and a default function that creates the engine for one workspace:

```js
export const meta = {
    api: 1,
    name: "row",
    actions: {
        "swap-with-first": { description: "Swap the focused window with the first one." },
    },
    options: {
        width: { type: "number", default: 0.5, description: "Window width, as a proportion." },
    },
};

export default function create(ws) {
    const windows = [];

    // Puts the windows side by side.
    function layout() {
        const gaps = ws.config.gaps;
        const area = ws.view.tilingArea;
        const w = ws.round(area.w * ws.options.width - gaps);
        windows.forEach((win, i) => {
            ws.place(win, { x: area.x + gaps + i * (w + gaps), y: area.y + gaps, w, h: area.h - gaps * 2 });
        });
    }

    return {
        windowAdded(win, how) {
            windows.push(win);
            layout();
            if (how.activate) ws.focus(win);
        },
        windowRemoved(win) {
            windows.splice(windows.indexOf(win), 1);
            layout();
            if (ws.focused === null && windows.length > 0) ws.focus(windows[0]);
        },
        action(name) {
            if (name !== "swap-with-first" || windows.length < 2) return false;
            const i = windows.indexOf(ws.focused);
            [windows[0], windows[i]] = [windows[i], windows[0]];
            layout();
            return true;
        },
    };
}
```

For a complete engine to start from, copy [`examples/template.js`](https://github.com/faetalize/bento/blob/main/resources/layouts/examples/template.js): under 300 lines, comments included, that put one main window on the left and stack the others on the right, with comments on every event, what bento does when an engine leaves one out, and tests next to it.

The full API, with every event an engine can handle, is in [`bento-layout.d.ts`](https://github.com/faetalize/bento/blob/main/resources/layouts/bento-layout.d.ts).
In short:

- **Events** tell the engine what happened: windows added and removed, actions, focus requests, fullscreen requests, window size changes, view and config changes, pointer resizes, swipe gestures and dropped windows.
- **The workspace** (`ws`) is everything an engine can do: place windows on the canvas, show and hide them, set fullscreen or maximized, focus, move the camera, highlight drop targets, and draw tab bars.
- **Queries** ask the engine without changing anything: where a dragged window would land, what size a new window should start at, which windows move along to another workspace, and positions for IPC.

bento animates everything an engine changes, so engines only say where things go.

### Testing layout engines

Engines can be tested with [node](https://nodejs.org), without running bento.
`resources/layouts/test/host.js` plays bento's role: it opens and closes windows, runs actions, drags and resizes, and shows where the engine puts everything.

```js
import { test } from "node:test";
import assert from "node:assert/strict";
import { TestHost } from "./host.js";

test("windows go side by side", async () => {
    // The path is relative to host.js.
    const host = await TestHost.load("../row.js");
    const a = host.open();
    const b = host.open();
    assert.deepEqual(host.rect(b), { x: 656, y: 16, w: 624, h: 688 });
    assert.equal(host.focused, b);
});
```

```sh
node --test resources/layouts/
```

### Limits

Engines run in a sandbox, so an engine from someone else can't do anything but arrange windows.
They have no access to files, the network or other modules, and each call has a time limit and a memory limit.

When an engine fails, bento keeps going:

- If a call throws, bento keeps what the engine did before the error and logs it.
- If a call runs out of time, bento starts the engine over for that workspace, from where the windows are.
- If the engine doesn't load, bento keeps the previous one, or falls back to a simple engine that puts windows side by side.
