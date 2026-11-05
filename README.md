> [!NOTE]
> **bento** is a fork of [niri](https://github.com/niri-wm/niri).
> Please don't report bento issues to the niri project.
> bento has no niri IPC socket: its own programs talk to it through `BENTO_SOCKET`, see [The socket](#the-socket).
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
For the settings app, also install libadwaita (`libadwaita-1-dev` on Ubuntu, `libadwaita-devel` on Fedora) and build it with `cargo build --release -p bento-settings`.

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
| `target/release/bento-settings` (settings app) | `/usr/local/bin/` |
| `resources/io.github.faetalize.bento.Settings.desktop` (settings app) | `/usr/local/share/applications/` |

Doing this will make bento appear in GDM and other display managers.

`bento.service` and the dinit service start `bento` by name rather than by full path, so put it in `/usr/local/bin/` or `/usr/bin/` where they'll find it.

Tools made for niri run the `niri` command, so link it to bento.
This also keeps them working if you have niri installed, since an older `niri` client can't read newer events:

```sh
sudo ln -s /usr/local/bin/bento /usr/local/bin/niri
```

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

bento reads niri's config format, but what belongs to niri's way of tiling isn't bento's: column widths, centering, tabbed columns, the tab indicator, the insert hint, window rules that size new windows, and binds to column actions like `focus-column-left`.
Those are the `niri` engine's now, and live in its own file, `~/.config/bento/engines/niri.kdl`.
A converter moves them there:

```sh
bento migrate-from-niri
```

It reads `~/.config/niri/config.kdl`, or the path you give it, and writes `~/.config/bento/config.kdl` and `engines/niri.kdl`, with `tiling { layout "niri" }` set.
Files that already exist are kept next to the new ones, with `.before-migration` added to their names.
Everything else in your config stays as you wrote it, comments included.
What can't be converted is dropped and listed when it finishes: gradients on the tab indicator and the insert hint, per-output and per-workspace values of the options that moved, window rules with `is-active-in-column` (no engine has it), and binds to actions bento doesn't have.
It doesn't follow `include`s, so move those by hand.
Run `bento validate` after it.

bento Settings does the same from the Engine page, under "Migrate from niri…", and shows what couldn't be converted before it writes anything.

When a session starts, bento links `~/.config/niri` to `~/.config/bento` if nothing is at that path, so tools that edit niri's config change bento's.
A real `~/.config/niri` stays untouched, and bento won't see what tools write there.
After linking, niri itself can't run from that config if it uses anything specific to bento, like `tiling {}`.

### Desktop shells

bento sets `XDG_CURRENT_DESKTOP=bento:niri`, so portals and tools that look for niri find it, but it has no niri IPC socket.
Desktop shells made for niri, such as [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) and [Noctalia](https://github.com/noctalia-dev/noctalia-shell), lose what they read from that socket.
Shells get the basics from Wayland protocols: `ext-workspace`, foreign-toplevel and output management.

The default bento config runs Waybar, so with a desktop shell you might get two bars on screen.
To fix this, stop Waybar with `pkill waybar` command, then open `~/.config/bento/config.kdl` and delete the `spawn-at-startup "waybar"` line.

### Settings

Bento Settings is a GTK app for changing bento's settings without editing the config.
Run `bento-settings`, or `bento-settings engine` to open a page directly, or `bento-settings --search gaps` to search.

It edits `~/.config/bento/config.kdl` and the layout engine's own file in place, and changes only the values you set, so your comments and layout stay as they are.
Its Shortcuts page lists every bind, bento's and the engine's, and records new ones by pressing them; binds from included files are shown but left alone.
It checks every change with bento's own parser before saving, and bento applies it right away.

### Wallpaper

bento can draw a wallpaper itself, without a separate tool like swaybg:

```kdl
layout {
    background-image "~/Pictures/wallpaper.jpg"
}
```

It fills each monitor, cropped to fit, and takes the place of `background-color`.
Set it per monitor with a `layout {}` section inside `output`.
A shell's own wallpaper draws above it, so turn that off to see bento's; in Noctalia, run `noctalia msg config-set wallpaper.enabled false`.

### Default applications

If you install bento alongside GNOME or KDE and want to keep their default app associations (e.g. which image viewer or video player to use), symlink their `mimeapps.list` like so:

```
ln -s /usr/share/applications/gnome-mimeapps.list ~/.local/share/applications/bento-mimeapps.list
```

## The socket

bento's own programs, which are `bento msg`, bento Settings and scripts, talk to it through one socket that carries JSON.
Its path is in `$BENTO_SOCKET`, which bento sets for everything it starts, and it works without a Wayland connection.
The format belongs to bento and can change at any time: it isn't niri's IPC.

Each request is one line of JSON with its kind in `"type"`, and bento answers it with one line.
Keys are camelCase.
A `subscribe` request turns the connection into a stream of events, so a program that wants both opens two connections.

```sh
echo '{"type":"get","what":"windows"}' | bento msg raw-request
echo '{"type":"action","name":"focus-workspace","args":[3],"properties":{}}' | bento msg raw-request
```

Requests:

- `get` reads one of `version`, `monitors`, `workspaces`, `windows`, `groups`, `layers`, `focused`, `keyboardLayouts`, `overview`, `casts`, `engine`, `engineState` and `actions`.
  Monitors, workspaces, windows and groups are bento's objects, with the same names layout engines see them by.
  `engine` is the running engine's name, actions with their arguments, options and window rules.
  `engineState` is whatever the running engine exposes about itself, in a shape of its own.
  `actions` is bento's own actions with their arguments.
- `action` runs an action by name with its arguments and properties, as written in a bind, and parses it the way the config does, so the same things are invalid.
  Besides bento's actions it runs the running engine's, by their names.
  `window` and `workspace` aim it at another window or workspace than the focused one.
- `output` changes a monitor's configuration temporarily, `pickWindow` and `pickColor` let the user pick, `loadConfigFile` reloads the config, `focusWindow`, `setWindowUrgent` and `stopCast` do what they say.
- `subscribe` takes the kinds of events to get, or all of them.
  A new subscriber first gets the current state as events like `monitorAdded` and `windowAdded`.
  The events are `monitorAdded`, `monitorChanged`, `monitorRemoved`, the same for `workspace`, `window` and `group`, `focusChanged`, `engineChanged`, `engineStateChanged`, `keyboardLayoutsChanged`, `keyboardLayoutSwitched`, `overviewChanged`, `castsChanged`, `configLoaded`, `screenshotCaptured` and `touchpadToggled`.

`bento msg` is the command line for all of it, with readable output and raw JSON with `-j`.
Its subcommands are named for the requests: `bento msg windows`, `bento msg engine-state`, `bento msg action focus-workspace 3`, `bento msg action -p focus=false move-window-to-workspace 2`, `bento msg subscribe windowAdded windowRemoved`.
`bento msg --help` lists them all.

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
Each one defines how the workspace works, with actions of its own.
So each engine has a config file of its own for its options and its binds, loaded only while that engine runs:

- `engines/bento.kdl` and `engines/niri.kdl` next to the config, so `~/.config/bento/engines/bento.kdl`, for the engines that ship with bento;
- the engine file's path with `.kdl` instead of `.js` for an engine file, so `~/.config/bento/layouts/spiral.kdl` for the one above.

The first time a built-in engine runs without its file, bento writes its defaults there, with binds for moving around and resizing windows; from then on the file is yours.
Switching engines switches which file is loaded, so nothing breaks.
A key bound in both the config and the engine's file is an error.

When the engine file changes, bento reloads it, just like the config.
Engines can keep their state across reloads, so your windows stay where they are while you work on an engine.

### Options

Engines declare their own options, set in `options` in the engine's file:

```kdl
// ~/.config/bento/engines/bento.kdl
options {
    gravity false
}
```

An option is its name followed by its values, and the engine declares the type of each:

- a boolean, `true` or `false`;
- a number;
- a string;
- a choice, one of the strings the engine lists;
- a size, written like `"50%"` for a proportion of the working area or `"800"` for logical pixels;
- or a list of any of those, written as several values: `preset-column-widths "33%" "50%" "67%"`.

A size written with no value, like `default-column-width` alone on its line, leaves it to the window.
`bento validate` checks the options against what the engine declares, and so does bento when it loads the config.
`bento msg engine` lists the running engine's options with their types and current values, and its actions and window rules, and `bento msg -j engine` gives the same as JSON for tools like settings screens.
bento Settings shows the running engine's options on its Engine page, with a field for each type.

What works under every engine is bento's and stays in the main config.
Engines get its values, such as `gaps` and `struts`, and decide what they mean.
Borders, focus rings and shadows look the way the `layout {}` section says, whatever the engine.
Anything that belongs to one way of tiling, like column widths, is an option of the engine that has it.

### Window rules

Window rules split the same way.
Rules that make sense under every engine, like `open-floating`, `opacity` or `block-out-from`, are bento's, in the main config.
Rules that belong to an engine's way of working are declared by the engine, and written in its file with the same `match` and `exclude`, and properties written like options:

```kdl
// ~/.config/bento/engines/niri.kdl
window-rule {
    match app-id="firefox"
    default-column-width "67%"
}
```

Where several rules match a window, later ones win, as in the main config.
`bento validate` reports properties the engine doesn't declare and values of the wrong type.

### Actions

Moving around and resizing tiled windows is the engine's: each engine has its own actions, named for what they do in it.
The `niri` engine has niri's, like `focus-column-left` or `consume-or-expel-window-left`, and the `bento` engine has its own, like `focus-left` or `stack-left`.
Bind them in the engine's file by name, followed by any arguments:

```kdl
// ~/.config/bento/engines/bento.kdl
binds {
    Mod+H { focus-left; }
    Mod+Minus { set-width "-10%"; }
}
```

The main config can't bind them, so its binds work whichever engine runs.
Everything else is bento's own and bound in the main config: spawning, screenshots, workspaces, monitors, the overview, and floating windows, which have their own actions like `focus-floating-left` and `move-floating-left`.

Run an engine's action from the command line with `layout-action`, followed by the action's name and any arguments:

```sh
bento msg action layout-action focus-left
```

`bento validate` checks that the engine has the actions your binds name, and that their arguments fit.
`bento msg engine` lists every action the running engine has, with what it does and the arguments it takes.

Engines only see their own workspace for now, so no engine action reaches another monitor or workspace; bento's own actions, like `focus-monitor-left` or `focus-workspace-down`, do that.

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

`meta.actions` lists every action the engine has, with a description of what it does.
An action can't take the name of one of bento's own actions, like `spawn`: the engine fails to load if it does.
An action that takes arguments says which, in order, so that bento can check binds and fill in defaults, and settings screens can offer the right field:

```js
"resize": {
    description: "Grow the focused window towards a side.",
    args: [
        { name: "cells", type: "number", default: 1 },
        { name: "side", type: "choice", choices: ["left", "right"] },
    ],
},
```

Argument types are `boolean`, `number`, `string`, `choice` (one of `choices`) and `size`, which is written like `"50%"`, `"+10%"` or `"800"` and reaches the engine as `{ op, value, unit }`.
Arguments with a `default` can be left out, and an action without `args` takes none.

For a complete engine to start from, copy [`examples/template.js`](resources/layouts/examples/template.js): under 300 lines, comments included, that put one main window on the left and stack the others on the right, with comments on every event, what bento does when an engine leaves one out, and tests next to it.

The full API, with every event an engine can handle, is in [`bento-layout.d.ts`](resources/layouts/bento-layout.d.ts).
In short:

- **Events** tell the engine what happened: windows added and removed, actions, focus requests, fullscreen requests, window size changes, view and config changes, pointer resizes, swipe gestures and dropped windows.
- **The workspace** (`ws`) is everything an engine can do: place windows on the canvas, show and hide them, set fullscreen or maximized, focus, move the camera, highlight drop targets, and draw tab bars.
- **Queries** ask the engine without changing anything: where a dragged window would land, what size a new window should start at, and positions for IPC.

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

Its actions are named for what they do on the grid, and `bento msg engine` lists them all with what each does.
`focus-left`, `focus-right`, `focus-up` and `focus-down` go to the nearest window that way, even diagonally, and `move-left` and the rest move the focused window one cell, swapping with a window in the way.
A window's stack is the windows covering the same column of cells: `stack-left` and `stack-right` stack a window under its neighbour or take it out, `focus-stack-top` and `focus-in-stack N` move within it, and `toggle-tabs` shows it as tabs.
`set-width`, `set-height`, `cycle-width` and `cycle-height` size the focused window in whole cells.
Actions like `focus-index N` count the places where windows start, from left to right starting at 1.

By default, windows slide left within their rows until they touch another window, and the grid has a fixed left edge.
You can turn this off with the engine's `gravity` option, in `~/.config/bento/engines/bento.kdl`:

```kdl
options {
    gravity false
}
```

Without gravity, windows stay where you put them, and only columns of cells that are completely empty between windows collapse.
The grid also has no left edge then, so windows can be pushed or moved further left than the first one.

It also has actions for two windows at a time:

- `split-h` splits a window's cells in two side by side, and gives one half to another window.
- `split-v` does the same with stacked halves.
- `swap` swaps two windows' places and sizes.

Each takes two presses.
The first press picks the focused window and highlights it.
Then focus another window as usual, and press again.
Pressing again on the window you picked first cancels.

`stuff` puts the focused window between two windows that touch, side by side or stacked.
The first press highlights where the window would go, at the seam nearest to it.
While the highlight is up, `focus-left`, `focus-right`, `focus-up` and `focus-down` move it to the next seam in that direction, and any other action cancels.
The second press puts the window there.

## The niri engine

The `niri` engine is niri's scrollable tiling as a bento layout engine, ported from niri's `src/layout/scrolling.rs` to JavaScript in [`resources/layouts/niri.js`](resources/layouts/niri.js).
Its functions keep niri's names where they do the same thing, so the two are easy to compare.

niri's layout actions, bound in `engines/niri.kdl`, the column options in its `options {}`, window rules that size and place new windows, and tabbed columns all work like in niri.
It differs in these ways:

- **Its settings are in its file.**
  `engines/niri.kdl` has `options {}` for `center-focused-column`, `always-center-single-column`, `default-column-width`, `preset-column-widths`, `preset-window-heights`, `default-column-display`, `tab-indicator` and its `tab-indicator-position`, `-width`, `-gap`, `-length`, `-corner-radius`, `-place-within-column`, `-hide-when-single-tab` and `-active-color`, `-inactive-color`, `-urgent-color`, and `insert-hint` with `insert-hint-color`.
  Its window rules set `default-column-width`, `default-window-height`, `default-column-display` and `open-maximized`.
  Colors are strings, so the tab indicator and the insert hint have no gradients.
- **Animations are bento's.**
  bento animates the windows and the view the same way for every engine, with the `window-movement` and `horizontal-view-movement` animations, so they can look slightly different from niri's own.
- **Actions stay on their workspace.**
  niri's actions that go on to another monitor or workspace at the edge, like `focus-column-or-monitor-left`, don't exist yet, because engines only see their own workspace for now.
  bento's own `focus-monitor-left` or `focus-workspace-down` do that part.
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
