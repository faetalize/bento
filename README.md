> [!NOTE]
> **bento** is a fork of [niri](https://github.com/niri-wm/niri).
> Please don't report bento issues to the niri project.
> bento keeps niri's IPC protocol and `NIRI_SOCKET`, so tools that talk to niri should keep working.
> Config lives in `$XDG_CONFIG_HOME/bento/config.kdl`.

<h1 align="center">bento</h1>
<p align="center">A Wayland compositor with scriptable tiling, based on niri</p>
<p align="center">
    <a href="https://github.com/faetalize/bento/blob/main/LICENSE"><img alt="GitHub License" src="https://img.shields.io/github/license/faetalize/bento"></a>
</p>

## About

bento is a fork of niri but with changes that don't necessarily follow the original author's vision. Please do not open issues on the original repository if you face any issues using bento.

This repo follows a more permissive development stride, and thus may be less stable than niri. Caaaareful. Kay?

This README covers what's specific to bento. For everything else, see [niri's documentation](https://niri-wm.github.io/niri/).

## Features

- Everything niri has, except that tiling is done by a layout engine
- Layout engines are JavaScript files that decide how a whole workspace behaves: where windows go, which window has focus, where the view points and what actions do. Point the config at one, and bento reloads it as you edit it. See [Layout engines](#layout-engines), and start from the [template](resources/layouts/examples/template.js) to write your own
- Engines run in a sandbox, with no access to files or the network, so you can try ones other people share
- Two engines ship with bento:
  - `bento`, the default: a grid of cells, 12 rows tall, that scrolls sideways. Windows cover rectangles of cells, push each other aside, and slide left with gravity. See [The bento engine](#the-bento-engine)
  - `niri`: niri's scrollable columns. See [The niri engine](#the-niri-engine)
- Support for more experimental Wayland protocols

## Installing

bento has no packages, so you need to build it from source.

First, install the dependencies for your distribution.

- Ubuntu 24.04:

    ```sh
    sudo apt-get install -y gcc clang libudev-dev libgbm-dev libxkbcommon-dev libegl1-mesa-dev libwayland-dev libinput-dev libdbus-1-dev libsystemd-dev libseat-dev libpipewire-0.3-dev libpango1.0-dev libdisplay-info-dev
    ```

- Fedora:

    ```sh
    sudo dnf install gcc libudev-devel libgbm-devel libxkbcommon-devel wayland-devel libinput-devel dbus-devel systemd-devel libseat-devel pipewire-devel pango-devel cairo-gobject-devel clang libdisplay-info-devel
    ```

Next, get latest stable Rust: https://rustup.rs/

Then, build bento with `cargo build --release`.

Check Cargo.toml for a list of build features.
For example, you can replace systemd integration with dinit integration using `cargo build --release --no-default-features --features dinit,dbus,xdp-gnome-screencast`.

> [!WARNING]
> Do NOT build with `--all-features`!
>
> Some features are meant only for development use.
> For example, one of the features enables collection of profiling data into a memory buffer that will grow indefinitely until you run out of memory.

The repository also has a flake, inherited from niri, with a devshell that provides the required dependencies.
Run `nix develop`, then build bento with `cargo build --release`.

Put the files in the directories indicated in the table below.
These may vary depending on your distribution.

| File | Destination |
| ---- | ----------- |
| `target/release/bento` | `/usr/local/bin/` |
| `resources/bento-session` | `/usr/local/bin/` |
| `resources/bento.desktop`  | `/usr/local/share/wayland-sessions/` |
| `resources/bento-portals.conf` | `/usr/local/share/xdg-desktop-portal/` |
| `resources/bento.service` (systemd) | `/etc/systemd/user/` |
| `resources/bento-shutdown.target` (systemd) | `/etc/systemd/user/` |
| `resources/dinit/bento` (dinit) | `/etc/dinit.d/user/` |
| `resources/dinit/bento.target` (dinit) | `/etc/dinit.d/user/` |

Doing this will make bento appear in GDM and other display managers.

`bento.service` and the dinit service start `bento` by name rather than by full path, so put it in `/usr/local/bin/` or `/usr/bin/` where they'll find it.

## Starting bento

After installing, start bento from your display manager like GDM.
Press <kbd>Super</kbd><kbd>T</kbd> to run a terminal ([Alacritty](https://github.com/alacritty/alacritty)) and <kbd>Super</kbd><kbd>D</kbd> to run an application launcher ([fuzzel](https://codeberg.org/dnkl/fuzzel)).
Press <kbd>Super</kbd><kbd>Shift</kbd><kbd>/</kbd> to see the important hotkeys, and <kbd>Super</kbd><kbd>Shift</kbd><kbd>E</kbd> to exit bento.

If you're not using a display manager, you should run `bento-session` (systemd/dinit) or `bento --session` (others) from a TTY.
The `--session` flag will make bento import its environment variables globally into the system manager and D-Bus, and start its D-Bus services.
The `bento-session` script will additionally start bento as a systemd/dinit service, which starts up a graphical session target required by some services like portals.

> [!WARNING]
> Running bento directly from a TTY means that exiting or crashing bento will put the user straight into an unlocked shell prompt.
> Set up a display manager to avoid this.

You can also run `bento` inside an existing desktop session.
Then it will open as a window, where you can give it a try.
Note that this windowed mode is mainly meant for development, so it is a bit buggy (in particular, there are issues with hotkeys).

If you get a black screen, or run bento on an NVIDIA GPU, in a virtual machine or on Asahi, see [niri's Getting Started](https://niri-wm.github.io/niri/Getting-Started.html).
Its advice applies to bento, with `~/.config/bento/config.kdl` in place of niri's config path.

### Coming from niri

bento reads niri's config format, so you can copy your niri config to `~/.config/bento/config.kdl`.
Without a `tiling {}` section you get the `bento` engine, so add this to keep niri's columns:

```kdl
tiling {
    layout "niri"
}
```

### Desktop shells

bento keeps niri's IPC and sets `XDG_CURRENT_DESKTOP=bento:niri`, so tools made for niri can work with bento, but their instructions are written for niri.
Complete desktop shells such as [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) and [Noctalia](https://github.com/noctalia-dev/noctalia-shell) work with bento through its IPC.

The default bento config runs Waybar, so with a desktop shell you might get two bars on screen.
To fix this, stop Waybar with `pkill waybar` command, then open `~/.config/bento/config.kdl` and delete the `spawn-at-startup "waybar"` line.

### Default applications

If you install bento alongside GNOME or KDE and want to keep their default app associations (e.g. which image viewer or video player to use), symlink their `mimeapps.list` like so:

```
ln -s /usr/share/applications/gnome-mimeapps.list ~/.local/share/applications/bento-mimeapps.list
```

## Layout engines

In bento, tiling is done by a layout engine: a script that decides how the whole workspace behaves.
bento itself only knows about windows and a camera over an unbounded canvas, and gives engines small primitives to work with.
The engine decides everything else: where tiled windows go, which window has focus, where the camera points, and what actions do.
Concepts like columns, cells or gravity belong to the engine, not to bento.

Pick an engine, or an engine file, in the `tiling` section:

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

For a complete engine to start from, copy [`examples/template.js`](resources/layouts/examples/template.js): under 300 lines, comments included, that put one main window on the left and stack the others on the right, with comments on every event, what bento does when an engine leaves one out, and tests next to it.

The full API, with every event an engine can handle, is in [`bento-layout.d.ts`](resources/layouts/bento-layout.d.ts).
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

## The bento engine

The `bento` engine, the default, puts tiled windows on a grid of cells.
Like niri's strip of columns, the grid scrolls horizontally without end, and every workspace has its own grid.
Unlike niri, a window doesn't have to fill a column from top to bottom.
Every window covers a rectangle of whole cells: a starting column and row, plus a span.
So windows can sit in different rows, stack on top of each other, and span several cells in either direction.

The grid has 12 rows.
The view is a multiple of 12 columns wide, like the rows: bento picks the multiple that makes cells closest to square, from the output size and the gaps.
So halves, thirds, quarters and sixths of the view are whole numbers of cells, in both directions.
Window sizes snap to whole cells.

bento keeps niri's action and option names, so many of them still say "column", like `move-column-left` or `default-column-width`.
Actions like `focus-column N` count the places where windows start, from left to right starting at 1.
Actions that work within a column, like `focus-window-top`, work on the windows covering the same column of cells.

By default, windows slide left within their rows until they touch another window, and the grid has a fixed left edge.
You can turn this off with the engine's `gravity` option:

```kdl
tiling {
    layout "bento"
    options {
        gravity false
    }
}
```

Without gravity, windows stay where you put them, and only columns of cells that are completely empty between windows collapse.
The grid also has no left edge then, so windows can be pushed or moved further left than the first one.

The `bento` engine has actions of its own for two windows at a time:

- `layout-action "split-h"` splits a window's cells in two side by side, and gives one half to another window.
- `layout-action "split-v"` does the same with stacked halves.
- `layout-action "swap"` swaps two windows' places and sizes.

Each takes two presses.
The first press picks the focused window and highlights it.
Then focus another window as usual, and press again.
Pressing again on the window you picked first cancels.

`layout-action "stuff"` puts the focused window between two windows that touch, side by side or stacked.
The first press highlights where the window would go, at the seam nearest to it.
While the highlight is up, `focus-column-left`, `focus-column-right`, `focus-window-up` and `focus-window-down` move it to the next seam in that direction, and any other action cancels.
The second press puts the window there.

## The niri engine

The `niri` engine is niri's scrollable tiling as a bento layout engine, ported from niri's `src/layout/scrolling.rs` to JavaScript in [`resources/layouts/niri.js`](resources/layouts/niri.js).
Its functions keep niri's names where they do the same thing, so the two are easy to compare.

niri's layout actions, the `layout {}` options that describe columns, window rules that size and place new windows, tabbed columns, and the IPC all work like in niri.
It differs in three ways:

- **Animations are bento's.**
  bento animates the windows and the view the same way for every engine, with the `window-movement` and `horizontal-view-movement` animations, so they can look slightly different from niri's own.
- **No actions of its own.**
  The engine has no `layout-action`s; everything goes through niri's actions.
- **Switching engines starts from where the windows are.**
  The engine keeps its columns when bento reloads it, but when you switch to it from another engine, it builds columns from the windows' positions.

Everything outside the tiled windows, like workspaces, monitors, floating windows and the overview, is bento's, the same as with every engine.

## Status

bento does not guarantee stability, but does most things expected of a Wayland compositor.

## Contributing

Not accepting contributions until 2027

## License

bento is licensed under GPL-3.0-or-later, the same as niri. See [LICENSE](LICENSE).

## Contact

@faetalize on Discord.

Join the discord server: https://discord.gg/EZFTEvWWxC
