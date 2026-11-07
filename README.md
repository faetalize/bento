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
For the shell, also install gtk4-layer-shell and PulseAudio's client library (`libgtk4-layer-shell-dev` and `libpulse-dev` on Ubuntu, `gtk4-layer-shell-devel` and `pulseaudio-libs-devel` on Fedora) and build it with `cargo build --release -p bento-shell`.

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
| `target/release/bento-shell` (shell) | `/usr/local/bin/` |
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
Press <kbd>Super</kbd><kbd>T</kbd> to run a terminal ([Alacritty](https://github.com/alacritty/alacritty)) and <kbd>Super</kbd><kbd>D</kbd> to open the shell's launcher.
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
The `layout {}` of an `output` or a `workspace` moves too, to an `output` or `workspace` block of the engine's file, and so do gradients and a window rule's `tab-indicator`.
What can't be converted is dropped and listed when it finishes: window rules with `is-active-in-column` (no engine has it), and binds to actions bento doesn't have.
It doesn't follow `include`s, so move those by hand.
Run `bento validate` after it.

bento Settings does the same from the Engine page, under "Migrate from niri…", and shows what couldn't be converted before it writes anything.

When a session starts, bento links `~/.config/niri` to `~/.config/bento` if nothing is at that path, so tools that edit niri's config change bento's.
A real `~/.config/niri` stays untouched, and bento won't see what tools write there.
After linking, niri itself can't run from that config if it uses anything specific to bento, like `tiling {}`.

### The shell

bento's shell, `bento-shell`, is the desktop around the workspaces: a glass bar along the top of each monitor with the workspaces, the clock, the tray and the status icons, and the calendar, quick settings, notifications and the volume indicator that grow out of it.
`toggle-launcher`, bound to <kbd>Super</kbd><kbd>D</kbd>, opens its launcher.

The config names the shell, and bento starts it, and starts it again if it exits:

```kdl
shell "bento-shell"
```

bento hands it a Wayland connection of its own, through `WAYLAND_SOCKET`. On that connection only, the `bento_shell_v1` protocol (`resources/bento-shell-v1.xml`) gives the shell bento's commands for it, like `toggle-launcher`, and glass: the shell gives bento rounded shapes for each of its surfaces, and bento refracts, blurs and lights what's behind them, melting shapes that come close into one piece.
Without a `shell` line, bento starts no shell.

### Other desktop shells

bento sets `XDG_CURRENT_DESKTOP=bento:niri`, so portals and tools that look for niri find it, but it has no niri IPC socket.
Desktop shells made for niri, such as [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) and [Noctalia](https://github.com/noctalia-dev/noctalia-shell), lose what they read from that socket.
Shells get the basics from Wayland protocols: `ext-workspace`, foreign-toplevel and output management.

To run one instead of bento's, name it with `shell`, or remove the `shell` line and start it with `spawn-at-startup`.

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

### Glass

What bento blurs behind windows, layer surfaces and popups, whether a `background-effect` rule asks for it or the app does, is glass: blurred, bent like through a lens near its rounded edges, with the edges catching the light.
The `blur {}` section sets it for everything, and the even blur, without glass's edge, is the `plain` material:

```kdl
blur {
    material "glass" // or "plain"
    passes 3
    offset 3.0
    noise 0.02
    saturation 1.5

    // How glass bends and lights what's behind it.
    glass {
        bezel 20             // how far in from its edge it curves, in logical pixels
        thickness 40         // how tall it is, in logical pixels
        refractive-index 1.5 // 1 doesn't bend at all
        dispersion 0.14      // how much more red bends than blue
        highlight 0.6        // how bright the light it reflects is
        sheen 0.5            // how much it spreads that light, from polished 0 to rough 1
        light-size 0.4       // how big the light looks, from a point 0 to a broad window 1
        light-angle 315      // where the light comes from, clockwise from the top
    }
}
```

Every setting left out keeps its default, shown here.

A rule can pick the other material for some windows or surfaces:

```kdl
window-rule {
    match app-id="^org\.gnome\.Nautilus$"
    background-effect {
        material "plain"
    }
}
```

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
- `output` changes a monitor's configuration temporarily, `pickWindow` and `pickColor` let the user pick, `loadConfigFile` reloads the config, `focusWindow`, `focusWorkspace`, `setWindowUrgent` and `stopCast` do what they say.
- `subscribe` takes the kinds of events to get, or all of them.
  A new subscriber first gets the current state as events like `monitorAdded` and `windowAdded`.
  The events are `monitorAdded`, `monitorChanged`, `monitorRemoved`, the same for `workspace`, `window` and `group`, `focusChanged`, `engineChanged`, `engineStateChanged`, `keyboardLayoutsChanged`, `keyboardLayoutSwitched`, `overviewChanged`, `castsChanged`, `configLoaded`, `screenshotCaptured` and `touchpadToggled`.

`bento msg` is the command line for all of it, with readable output and raw JSON with `-j`.
Its subcommands are named for the requests: `bento msg windows`, `bento msg engine-state`, `bento msg action focus-workspace 3`, `bento msg action -p focus=false move-window-to-workspace 2`, `bento msg subscribe windowAdded windowRemoved`.
`bento msg --help` lists them all.

## Layout engines

In bento, tiling is done by a layout engine: a script that runs the presentation of the whole session.
bento keeps the objects, monitors, workspaces, windows and groups, and each workspace has a camera over an unbounded canvas.
The engine decides everything else: where tiled windows go, which window has focus, where each camera points, what actions do, and what's drawn between the windows, like tab bars and hints.
bento runs one instance of it for the whole session, which sees every monitor, workspace and window, so crossing from one to another is just the engine's own logic.
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
- a color, written `"#rrggbb"` or `"#rrggbbaa"`, or `"#rgb"`;
- a gradient, written with properties instead of values, like niri's: `from` and `to` colors, and optionally an `angle` in degrees (0 goes up, 90 to the right, and 180, top to bottom, by default), `relative-to` (`"node"`, the default, or `"view"` for the whole view of the workspace) and `in`, the color space to mix in (`"srgb"`, `"srgb-linear"`, `"oklab"`, or `"oklch"` with `"shorter hue"`, `"longer hue"`, `"increasing hue"` or `"decreasing hue"`): `active-gradient from="#80c8ff" to="#c7ff7f" angle=45 relative-to="view" in="oklch shorter hue"`;
- or a list of any of those but a gradient, written as several values: `preset-column-widths "33%" "50%" "67%"`.

A size written with no value, like `default-column-width` alone on its line, leaves it to the window.
So does a gradient: written alone, it takes the gradient away.
`bento validate` checks the options against what the engine declares, and so does bento when it loads the config.
`bento msg engine` lists the running engine's options with their types and current values, and its actions and window rules, and `bento msg -j engine` gives the same as JSON for tools like settings screens.
bento Settings shows the running engine's options on its Engine page, with a field for each type.

Options can differ per output and per named workspace, like the settings of `layout {}`.
An `output` or `workspace` block in the engine's file holds the options that change there:

```kdl
// ~/.config/bento/engines/niri.kdl
options {
    default-column-width "50%"
}

output "DP-2" {
    options {
        default-column-width "33.333%"
    }
}

workspace "web" {
    options {
        default-column-width "100%"
    }
}
```

A workspace has the options of `options {}`, then those of its output, then its own by name, and each replaces the one before it option by option.
Outputs are matched like they are in the main config, by connector, or by make, model and serial; workspaces by their name, ignoring case.
Engines read a workspace's options from the workspace, and `bento.options` stays the top-level ones.

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
The `niri` engine has niri's, like `focus-column-left` or `consume-or-expel-window-left`, and the `bento` engine has its own, like `focus-left` or `stack-from-right`.
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

Grouping is bento's too. A group is a set of windows that engines can address together, and which one is active, nothing more: it doesn't mean tabs, or one place in the layout, or that only one of its windows shows. How it looks is up to the engine, which can show tabs, a carousel or anything else.
`group-window-left`, `group-window-right`, `group-window-up` and `group-window-down` group the focused window with the nearest one in that direction, by where the engine put them, and `ungroup-window` takes it out.
Engines can group windows too, and read the groups, through `bento.groups` and `bento.createGroup`.
Floating windows can't be in a group, and a window that closes, floats or is dragged out leaves its group.

Searching windows is bento's too. An engine starts a search over a group's windows, a workspace's, or every window with `bento.search`, and bento keeps the query, takes the typing, and matches it against the windows' titles and app ids: whole words first, then prefixes, then substrings, then letters in order, and not at all windows that don't match.
While it runs the keyboard goes to the search's field, a node that bento draws and the engine places and styles, and apps get no keys; binds that can't be inhibited still run.
Typing, Backspace, Delete, the arrow keys, Home, End, Shift for selecting, Ctrl+A, Ctrl+C, Ctrl+X and Ctrl+V work in it, and screen readers see it as a text input.
Enter picks the best match, which reaches the engine as an `activaterequest` for that window, and Escape cancels.
Text from an input method can't be typed into it yet.

Run an engine's action from the command line with `layout-action`, followed by the action's name and any arguments:

```sh
bento msg action layout-action focus-left
```

`bento validate` checks that the engine has the actions your binds name, and that their arguments fit.
`bento msg engine` lists every action the running engine has, with what it does and the arguments it takes.

Engines see every monitor and workspace, so their actions can go from one to another.
The `niri` engine has `focus-column-or-monitor-left` and `move-column-to-workspace-down`, and the `bento` engine has `focus-left-or-monitor` and `move-down-or-to-workspace`.
bento's own actions, like `focus-monitor-left` or `move-window-to-workspace-down`, do the same for one window, whatever the engine.

### Writing a layout engine

A layout engine is a single JavaScript module.
It exports `meta`, which describes the engine, and a default function that creates it, once for the whole session.
The engine learns what happens from events on bento's objects, the way the DOM works, and changes the objects with method calls:

```js
export const meta = {
    api: 2,
    name: "row",
    actions: {
        "swap-with-first": { description: "Swap the focused window with the first one." },
    },
    options: {
        width: { type: "number", default: 0.5, description: "Window width, as a proportion." },
    },
};

export default function create(bento) {
    // The windows of each workspace in order, by workspace id.
    const rows = new Map();
    const rowOf = (ws) => {
        if (!rows.has(ws.id)) rows.set(ws.id, []);
        return rows.get(ws.id);
    };

    // Puts the windows of a workspace side by side.
    function layout(ws) {
        const gaps = ws.config.gaps;
        const area = ws.tilingArea;
        const w = ws.monitor.round(area.w * bento.options.width - gaps);
        rowOf(ws).forEach((win, i) => {
            win.place({ x: area.x + gaps + i * (w + gaps), y: area.y + gaps, w, h: area.h - gaps * 2 });
        });
    }

    function onRemove(e) {
        const ws = e.target.workspace;
        const windows = rowOf(ws);
        windows.splice(windows.indexOf(e.target), 1);
        layout(ws);
        if (ws.focused === null || ws.focused === e.target) windows[0]?.focus();
    }

    bento.addEventListener("windowadd", (e) => {
        rowOf(e.window.workspace).push(e.window);
        e.window.addEventListener("remove", onRemove);
        if (e.activate) e.window.focus();
        layout(e.window.workspace);
    });

    bento.addEventListener("action", (e) => {
        if (e.name !== "swap-with-first" || e.window === null) return;
        const windows = rowOf(e.window.workspace);
        const i = windows.indexOf(e.window);
        [windows[0], windows[i]] = [windows[i], windows[0]];
        layout(e.window.workspace);
    });
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

Argument types are `boolean`, `number`, `string`, `choice` (one of `choices`), `size`, `workspace` and `monitor`.
A size is written like `"50%"`, `"+10%"` or `"800"` and reaches the engine as `{ op, value, unit }`.
A workspace is written as an index on the focused monitor, counting from 1, or a name, and a monitor as a connector name like `"DP-2"`; the engine gets the object, or `null` when there isn't one.
Arguments with a `default` can be left out, and an action without `args` takes none.

An engine can declare window rules too, in `meta.windowRules`, which are written in its file and reach the engine as each window's `rules`.
Options and rules of type `size` reach it as `{ proportion }` or `{ fixed }`, those of type `color` as the `"#rrggbb"` string, and those of type `gradient` as `{ from, to, angle, relativeTo, in }` with what was written, or `null`.
A color or a gradient goes straight into a node's `background`, or the `color` of its `border`.
An engine reads its options from `bento.options`, or for a workspace from `workspace.options`, which has what the engine's file sets for the workspace's output and for the workspace by name on top.

For a complete engine to start from, copy [`examples/template.js`](resources/layouts/examples/template.js): under 250 lines, comments included, that put one main window on the left and stack the others on the right, with a line drawn between them, comments on every event, what bento does when an engine leaves one out, and tests next to it.

The full API, with every event an engine can handle, is in [`bento-layout.d.ts`](resources/layouts/bento-layout.d.ts).
In short:

- **Objects** are what the engine reads and changes: monitors, their workspaces, windows, each workspace's camera, and groups. Properties are read-only, and every change is a method call that says whether it worked, like `win.place(rect)`, `win.focus()`, `win.moveTo(workspace)` or `workspace.camera.move({ x })`.
- **Events** tell the engine what happened, on the object they concern: `windowadd` on `bento`, `remove` or `moderequest` on a window, `change` on a monitor or workspace, swipes and drags over a workspace, resizes and double clicks on a window's edges, and `action` when a bind runs one of the engine's actions.
- **Groups** are bento's own: a set of windows that engines can address together, and which one is active. Its windows stay ordinary windows. Any engine can use them, `bento.createGroup` makes one, and native group binds work under every engine. How a group looks is the engine's.
- **Search** is bento's own: `bento.search(scope)` starts it, and the engine gets a `Search` with the query, the matching windows best first, and the field to place in a tree, and hears `change` and `end`.
- **Nodes** are what the engine draws: boxes, text and images in a tree for each workspace, which also holds the windows. Tab bars, hints and drop targets are nodes. bento renders the tree and runs the transitions the engine describes, so no engine code runs per frame.
- **State** can be exposed with `bento.exposeState`, so that the shell, bento Settings and scripts can read it through bento's socket, and `unload` can keep state for the next instance of the engine when it reloads.
- **Timers and the keyboard**: `bento.setTimeout` calls back after a delay, between events, and ends with the engine. `bento.grabKeyboard()` takes the keyboard for a short interaction the person started, like picking a window by the letter it shows: keys come as `keydown` and `keyup` with their name, the text they type and the modifiers held, and apps get none until `releaseKeyboard()`, Escape (unless the engine prevents it), a click outside what the engine draws, the lock screen or keyboard focus moving elsewhere ends it, which `keyboardrelease` says. Outside a grab, an engine only hears `modifierschange`.
- **Queries** ask the engine without changing anything, like `newwindowsize`, what size a new window should start at.

bento animates everything an engine changes, so engines only say where things go.
Engines written for API 1, with `api: 1` and one instance for each workspace, still run for now, see [`bento-layout-v1.d.ts`](resources/layouts/bento-layout-v1.d.ts).

### Testing layout engines

Engines can be tested with [node](https://nodejs.org), without running bento.
`resources/layouts/test/host.js` plays bento's role: it opens and closes windows, runs actions, drags and resizes, types keys, lets time pass for timers, and shows where the engine puts everything.

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
    assert.equal(host.focused(), b);
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
- If a call runs out of time, bento starts the engine over, from where the windows are.
- If the engine doesn't load, bento keeps the previous one, or falls back to a simple engine that puts windows side by side.

## The bento engine

The `bento` engine, the default, puts tiled windows on a grid of cells.
Like niri's strip of columns, the grid scrolls horizontally without end, and every workspace has its own grid, with cells that fit its monitor.
Unlike niri, a window doesn't have to fill a column from top to bottom.
Every window covers a rectangle of whole cells: a starting column and row, plus a span.
So windows can sit in different rows, stack on top of each other, and span several cells in either direction.

The grid has 12 rows.
The view is a multiple of 12 columns wide, like the rows: bento picks the multiple that makes cells closest to square, from the output size and the gaps.
So halves, thirds, quarters and sixths of the view are whole numbers of cells, in both directions.
Window sizes snap to whole cells.

Its actions are named for what they do on the grid, and `bento msg engine` lists them all with what each does.
`focus-left`, `focus-right`, `focus-up` and `focus-down` go to the nearest window that way, even diagonally, and `move-left` and the rest move the focused window one cell, swapping with a window in the way.
In a group, `focus-left` and `focus-right` first go through its windows, and `move-left` and `move-right` reorder them.
A window's stack is the windows covering the same column of cells: `stack-from-right` stacks the window on the right under the focused one and `unstack` takes the focused one out, `focus-stack-top` and `focus-in-stack N` move within it, and stacking never groups windows.
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

The engine's other options are in its file too, written at their defaults.
`center-focused-window` and `always-center-single-window` say when the view centers the focused window, `default-width` is the width new windows open with, and `preset-widths` and `preset-heights` are what `cycle-width` and `cycle-height` go through.
`hint-color` is the color of the hints.
Its window rules set `default-width`, `default-height` and `open-full-width`.

`group` groups windows, as a bento group: while you hold the keys of its bind, each press picks the focused window, or unpicks it, and letting go of them groups the picked windows where the first one is.
Picking one window of a group alone takes it out, and picking windows of other groups moves them.
A group shares its area as a carousel: the active window takes most of it, and the others show as slits of their middle beside it, in order, half of the gaps apart.
Every window of a group is as large as the active one, so that activating one only moves it.
An area is always wide enough for the smallest width its windows accept, with a group's slits beside it, and pushes the windows after it out of the way.
Pointing at a slit widens it, and clicking it activates its window.
A group made by bento's own group actions shows as a carousel in the area of its active window, and while a group is fullscreen or maximized, only its active window shows.
Dragging a window shows the targets around the pointer: a round dot inside each edge of a window, which splits it and gives the dragged window the half on that side, a pill on each edge two windows share, which stuffs the dragged window between them, and a pill between two windows of a group, which it joins there.
Targets sit on the windows rather than in the gaps, so they work without gaps too.
Away from them, it shows the cells the window would land on.
Double-clicking the left or right edge of a window toggles its full width, and the top or bottom edge its full height.
Swiping sideways scrolls the view, and snaps it to window edges when the fingers lift.

It also has actions for two windows at a time:

- `split-h` splits a window's cells in two side by side, and gives one half to another window.
- `split-v` does the same with stacked halves.
- `swap` swaps two windows' places and sizes.

Each takes two presses.
The first press picks the focused window and highlights it with a node the engine draws.
Then focus another window as usual, and press again.
Pressing again on the window you picked first cancels.

`stuff` puts the focused window between two windows that touch, side by side or stacked, or between two windows of a group, which it joins.
The first press highlights the window and shows a pill on each place it can go, the one nearest to it picked.
`focus-left`, `focus-right`, `focus-up` and `focus-down` pick the next place in that direction, and any other action cancels.
The second press puts the window there.

Where the grid ends, actions go on to the monitor or workspace beside it:

- `focus-left-or-monitor` and `focus-right-or-monitor` focus the monitor to the left or right, and `focus-up-or-workspace` and `focus-down-or-workspace` the workspace above or below, when there is no window that way.
- `move-left-or-to-monitor` moves a window to the monitor to the left once it can't move left, and `move-up-or-to-workspace` and `move-down-or-to-workspace` to the workspace above or below once it can't move up or down. The grid has no end to the right, so there is no such action for that side.
- `move-to-workspace N` and `move-to-monitor "DP-2"` move the focused window, with its group, to a workspace of the focused monitor or the active workspace of a monitor.

Focus goes along with what moves.
bento's own `move-window-to-workspace` actions move one window and leave its group.
The engine's file binds the moves on `Mod+Ctrl+Shift`, which the main config leaves free, and has the focus actions as examples to bind on your focus keys.

## The niri engine

The `niri` engine is niri's scrollable tiling as a bento layout engine, ported from niri's `src/layout/scrolling.rs` to JavaScript in [`resources/layouts/niri.js`](resources/layouts/niri.js).
Its functions keep niri's names where they do the same thing, so the two are easy to compare.

niri's layout actions, bound in `engines/niri.kdl`, the column options in its `options {}`, window rules that size and place new windows, and tabbed columns all work like in niri.
It differs in these ways:

- **Its settings are in its file.**
  `engines/niri.kdl` has `options {}` for `center-focused-column`, `always-center-single-column`, `default-column-width`, `preset-column-widths`, `preset-window-heights`, `default-column-display`, `tab-indicator` and its `tab-indicator-position`, `-width`, `-gap`, `-gaps-between-tabs`, `-length`, `-corner-radius`, `-place-within-column`, `-hide-when-single-tab`, the colors `-active-color`, `-inactive-color` and `-urgent-color`, and the gradients `-active-gradient`, `-inactive-gradient` and `-urgent-gradient`, and `insert-hint` with the color `insert-hint-color` and the gradient `insert-hint-gradient`.
  A gradient wins over the color next to it, as in niri, and none is set by default.
  A gradient's `relative-to` is `"node"`, the tab or the hint, or `"view"`, where niri says `"window"` or `"workspace-view"`.
  Its window rules set `default-column-width`, `default-window-height`, `default-column-display` and `open-maximized`, and the colors and gradients of the window's tab, as niri's `tab-indicator` rule does: `tab-indicator-active-color`, `-inactive-color`, `-urgent-color`, `tab-indicator-active-gradient`, `-inactive-gradient` and `-urgent-gradient`.
  A window's rule comes before the options, and in each a gradient before the color.
- **Its options can differ per output and workspace.**
  Like niri's `layout {}` in an `output` or a `workspace`, the file's `output "DP-2" { options { … } }` and `workspace "web" { options { … } }` set the options for one output or named workspace, see [Options](#options).
  Every option of the engine can differ there, and each workspace has its own strip, so they apply where the workspace is.
- **Animations are bento's.**
  bento animates the windows and the view the same way for every engine, with the `window-movement` and `horizontal-view-movement` animations, so they can look slightly different from niri's own.
- **Actions that cross an edge take the whole column.**
  `focus-column-or-monitor-left`, `focus-window-or-workspace-down`, `move-column-to-workspace`, `move-column-to-monitor-left` and the rest of niri's go on to the monitor or workspace beside, and a trailing `false`, like `move-column-to-workspace 2 false`, keeps the focus where it is, as niri's `focus=false` does.
  The engine's file binds the moves to another workspace on `Mod+Ctrl+Shift`, and has the moves to another monitor as examples: bento's own binds for moving one window use the keys niri uses for them.
- **Tabbed columns are groups.**
  A tabbed column's windows are one bento group, so a group made by bento's own group actions shows as a tabbed column, and the tab indicator and the insert hint are drawn by the engine, following the `tab-indicator-*` and `insert-hint*` options.
  Stacked columns stay the engine's own.
- **Gestures and edges.**
  Swiping sideways scrolls the view and snaps to a column with the swipe's speed, and double-clicking the left or right edge of a window toggles its column's full width, and the top or bottom edge the window's full height.
- **Switching engines starts from where the windows are.**
  The engine keeps its columns when bento reloads it, but when you switch to it from another engine, it builds columns from the windows' positions.
- **Its state is readable.**
  The engine exposes its columns for each workspace, with their windows, widths, display and active window, for the shell and scripts to read through bento's socket.

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
