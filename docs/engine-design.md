# Engines, objects and actions

How layout engines and bento split the work. This holds the current decisions; open questions
are at the end.

## Terms

- **bento**: the compositor, a fork of niri.
- **niri**: the upstream compositor, where bento's native actions come from.
- **Engine**: a `*.js` layout engine, like `bento.js`, `niri.js` or a user's own file.

## Why this is changing

Today bento starts one copy of the engine per workspace, and each copy only sees its own
workspace: its windows, its view and its camera. Monitors and other workspaces don't exist for
it. So:

- bento sends niri's action names to every engine whether the engine declared them or not, and
  each engine gives them its own meaning. `bento.js`'s `move-column-left` moves one window by one
  cell, which isn't what its name says.
- Anything that crosses a workspace's edge, like `focus-column-or-monitor-left`, is done by
  bento in Rust after the engine returns false.
- Nothing can list everything an engine does, so a shortcuts page or `bento validate` has
  nothing complete to read.

## Decisions

### Engines run the presentation

Engines control window placement, layout, focus and the camera. bento doesn't take over after an
engine's action: there are no fallbacks. What an action does is all in the engine's code.

bento keeps:

- defining the objects: monitors appearing and leaving, their geometry and scale, windows'
  surfaces and size limits;
- the workspace structure and floating, below;
- panels and other layer-shell surfaces, the lock screen and popups;
- drawing: animations, borders, focus rings and shadows, and fullscreen windows above panels;
- the mechanics of input and pointer grabs, while the engine decides what a move or resize
  does.

### Primitives are objects

bento defines the objects, and engines address them:

- **Monitor**
- **Workspace**
- **Window**
- **Camera**
- possibly a few more.

A primitive is the definition of one of these objects, not an action helper like
`focusMonitor`.

Engines read objects through properties, which are read-only. Every change is a method call,
which can take options like `{ animate: false }` and returns whether it worked:

```js
win.place({ x: 0, y: 0, w: 800, h: 600 });          // false if the window refuses the size
workspace.camera.move({ x: 1200 }, { animate: false });
```

A change in a compositor is never just setting a value: it animates, tells the app, and can be
refused. A method call makes that visible, where a setter would hide it.

Some changes are carried out by bento after the engine's call returns, like moving a window or a
workspace, showing or renaming a workspace, or changing a group. Their methods return whether the
request is valid as the engine sees it. If bento then refuses one, for example because the target
workspace went away, the engine gets the event that undoes it, like the window's `move` back, so
its own state stays right.

The objects, as an engine sees them:

```js
class Bento {                    // what create(bento) receives
    get monitors() {}            // Monitor[]
    get focused() {}             // the focused Window, or null
    get config() {}              // bento's native settings: gaps, borders...
    get options() {}             // this engine's options, from its .kdl file

    window(id) {}                // find a Window by id
    log(...args) {}
}

class Monitor {
    get name() {}                // "DP-2"
    get size() {}                // { w, h }
    get scale() {}
    get workingArea() {}         // minus panels
    get workspaces() {}          // its vertical stack, top to bottom
    get activeWorkspace() {}
    get left() {}                // neighbouring Monitor, or null
    get right() {}
    get up() {}
    get down() {}

    round(value) {}              // snap to physical pixels on this monitor
}

class Workspace {
    get id() {}
    get name() {}
    get monitor() {}
    get windows() {}             // Window[], bottom to top
    get camera() {}              // Camera
    get active() {}              // shown on its monitor?

    show() {}                    // switch its monitor to it
    moveTo(monitor, index) {}    // move it to another monitor, or within its stack
    rename(name) {}
}

class Window {
    get id() {}
    get appId() {}
    get title() {}
    get workspace() {}
    get rect() {}                // where the engine last placed it
    get size() {}                // the size it actually took
    get minSize() {}
    get maxSize() {}
    get borders() {}
    get mode() {}                // "normal" | "fullscreen" | "maximized"
    get visible() {}
    get urgent() {}
    get data() {}                // anything the engine stored on it

    place(rect, { animate }) {}
    focus() {}
    raise() {}
    moveTo(workspace) {}
    setMode(mode) {}
    setVisible(visible) {}
    setData(data) {}
}

class Camera {                   // its own object, so it can grow, e.g. zoom
    get x() {}
    get y() {}

    move({ x, y }, { animate }) {}
}
```

There's no `setActiveInColumn`: it only served niri's `is-active-in-column` window rule, which
belongs to `niri.js`.

### Engines subscribe to events

Engines learn about changes by listening for events on the objects they concern, the way the
DOM works:

```js
export default function create(bento) {
    bento.addEventListener("windowadd", (e) => { ... });
    monitor.addEventListener("remove", (e) => { ... });
}
```

An engine only listens to what it uses.

### One engine for the whole session

The config names one engine, and bento runs one instance of it for the whole session. That
instance sees every monitor, workspace and window, and keeps its state for all of them, for
example a grid per workspace. Crossing from one monitor or workspace to another is just the
engine's own logic, with no handing over between copies.

### bento owns the workspace structure

Workspaces are arranged as in niri: each monitor has a vertical stack of them, and bento decides
which workspace is above or below, and when an empty workspace appears or goes away. bento also
shows them: switching between workspaces, its animation, and the overview.

Engines still address workspaces as objects. They can switch to one, move one to another
monitor or within its stack, rename one, and move windows onto one. Because one engine instance
sees every workspace, moving a window between workspaces is just the engine changing where the
window is, with no handing over.

### bento owns floating

Floating is the exception to tiling: a place for popups, dialogs and anything short-lived that
shouldn't disturb the layout. bento handles it the same way in every engine: where floating
windows go and how they move. So it works the same everywhere, and doesn't depend on the engine
getting dialogs right.

Moving through floating windows with the keyboard is bento's too: bento has its own actions to
focus, move and resize floating windows, like `focus-floating-left` and `move-floating-left`,
which work under every engine and are bound on their own keys. Engine actions only act on what
the engine places, and no action checks which layer has focus to decide where it goes.

An engine can turn the floating layer off. An engine that is itself about free placement, like
a desktop-style `windows11.js`, places every window itself, and a second floating layer under
it would only confuse people about which layer a window is in.

A window asks to float when it's a dialog, when an `open-floating` window rule matches it, or
when `toggle-window-floating` runs on it. With floating off, these requests go to the engine,
with the reason (a dialog and its parent, the size it asked for, a rule, the action). Most of
the time the engine does nothing with them. The binds and rules stay valid under every engine.

### Engines own fullscreen and maximize

A window asking to go fullscreen or maximized, or an action like `fullscreen-window`, is a
request to the engine (`modeRequested`). The engine tells the window its mode and places it
however its paradigm wants: `niri.js` keeps a fullscreen window as a column in the strip that
can be scrolled away from, and `bento.js` keeps its cells so it goes back to its place. bento
draws a fullscreen window above panels.

### bento's native config works under every engine

bento only ships what makes sense under every engine: spawning, screenshots, workspaces,
monitors, floating, gaps, borders and the like. Native binds and options never fail validation
because of the engine. If something only works with some engines, it doesn't belong in bento.

Everything that belongs to a paradigm comes from the engine that declares it. niri's column
actions (`focus-column-left`, `move-column-left`, `consume-or-expel-window-left`...) and the
column parts of `layout {}` (`preset-column-widths`, `default-column-width`,
`center-focused-column`...) become `niri.js`'s own actions and options. `bento validate`
complains only when the config uses something the running engine doesn't declare.

### Engines declare every action they have

`meta.actions` lists all of an engine's actions, and bento only sends an engine the actions it
declared. No action names are shared between engines: `niri.js` declares niri's column actions,
and `bento.js` declares names that say what it does.

Each declaration has a description and the action's arguments, with their names, types,
defaults and allowed choices, so bento Settings can offer the right field for each one:

```js
actions: {
    "resize": {
        description: "Grow the focused window towards a side.",
        args: [
            { name: "cells", type: "number", default: 1 },
            { name: "side", type: "choice", choices: ["left", "right", "up", "down"] },
        ],
    },
}
```

In an engine's `.kdl` file, its actions are written like native ones, by bare name:
`Mod+R { resize 2 "right"; }`. An engine action named like a native action is an error when the
engine loads.

### Engines declare their own window rules

Window rules split like binds. Rules that make sense under every engine, like `open-floating`,
`opacity` or `block-out-from`, are bento's. Rules that belong to a paradigm, like
`default-column-width` or `is-active-in-column`, are declared by the engine, written in its
`.kdl` file and checked by `bento validate`, the same as its actions and options.

### Engines own horizontal swipes

A horizontal swipe goes fully to the engine: bento only reports how the fingers move, and the
engine moves the camera, or does anything else, with it. Vertical swipes switch workspaces,
which is bento's.

### Engines are told when monitors change

Engines learn when a monitor appears, goes away, or changes its size, scale or working area.
What an engine does about it is up to the engine.

### Engines draw their own surfaces

Hints aren't a bento concept: some engines won't have them. Instead, engines can draw anything
they want: create surfaces anywhere, animate and morph them, make them interactive, and set
their depth. Hints, tab bars, carousels, thumbnails and drop targets are all things an engine
draws with this.

It works like a browser: the engine describes its surfaces as a tree it keeps changing, and
bento renders them. bento also runs their animations, from transitions the engine describes
(like CSS transitions), so no engine code runs per frame and a slow engine can't make the
desktop stutter.

Windows are nodes in the same tree as the surfaces an engine draws. Depth, transforms, clipping,
morphing and transitions work the same for a window as for anything else the engine draws, so a
carousel of windows is just the engine moving window nodes.

Each workspace has its own tree. Workspaces and monitors aren't nodes: bento composes the
workspaces itself, since switching between them and the overview are bento's, and an engine
transforming a workspace would fight bento's animations.

bento draws the tree with its own renderer, on smithay's, and borrows little else from a
browser. Nodes come in a few kinds: boxes (a fill, a border and a corner radius), text, images,
and windows. Any node can have a transform, opacity, clipping and transitions. Pointer events hit
the topmost node under the pointer and bubble up to its parents, like the DOM's. There's no layout
system, since engines do their own layout, and no browser engine underneath.

Engines only ever draw inside workspaces. Anything above them on a monitor, like a panel, an OSD
or a mini-map of an engine's columns, is the shell's job, and the shell can read the engine's
state to draw it.

### Grouping is bento's, its presentation is the engine's

bento has a native concept of a group of windows, which any engine can use. It's called
grouping rather than tabbing, because how a group looks is entirely up to the engine: tabs, a
carousel, thumbnails, or anything else.

A group is a set of windows and which one of them is active. Nothing more: order, looks and
layout are the engine's.

Which windows are grouped is bento's state, not the engine's. Engines can call on bento to
change it, like `bento.createGroup([a, b])`, `group.add(win)`, `group.remove(win)` and
`group.activate(win)`, and native group binds work under every engine. An engine reads the
groups from bento and knows how to deal with them, but doesn't keep track of them itself.

### An engine's state is readable from outside

The state an engine keeps, like `niri.js`'s columns, is accessible to the rest of the desktop:
the shell, bento Settings and scripts can read it directly. Nothing has to translate an engine's
concepts into bento's for them to be seen.

Engine state isn't declared. Unlike an option, which always maps to a control whatever it
means, state only makes sense to code that knows the engine, and that code learns its shape
from the objects themselves. Engines expose their state as they like.

### bento becomes a desktop with its own shell

bento ships its own shell (panel, launcher, notifications, OSDs) next to bento Settings, rather
than relying on third-party shells built for niri.

The shell is its own program, a Wayland client like any other, not part of bento. It can use
any toolkit, a crash only takes down the shell, and it uses the same protocols as every other
app: for example, background effects (blur radius, glass shaders) are a bento protocol that the
shell and any app use alike, while the shaders themselves live in bento, which is the only
place that can see what's behind a surface.

### A private socket for state and actions

bento's own programs (the shell, bento Settings, the CLI and scripts) talk to bento through one
socket, carrying JSON: bento's objects, engine state as the engine exposes it, events, and
actions. JSON fits engine state, which isn't declared. Any toolkit or language can use it
without bindings, it can be inspected with `bento msg -j`, and it works without a Wayland
connection.

The socket's format belongs to bento and its own programs, and can change at any time.
Compatibility with niri's IPC isn't a goal, and bento has no niri IPC socket: shells written for
niri, like noctalia, lose what they read from it. Third-party shells get the basics through
standard Wayland protocols (`ext-workspace`, foreign-toplevel, output management).

Wayland protocols are only for things tied to a surface, like background effects, since only
Wayland knows which window is which.

### A failing engine falls back

An engine that fails to load, throws while it starts, or fails three calls in a row is replaced by
the fallback engine, which places windows side by side and has no actions, until its file
changes. The error shows in `bento validate`, `bento msg engine` and bento Settings.

### Old engines keep working for a while

The object model is a new API version. bento keeps running `api: 1` engines for a while
alongside it.

### Moving from niri is a one-off conversion

bento keeps no niri spellings natively and no aliases for them. Instead, a converter turns a
niri `config.kdl` into a bento config. bento Settings gets a "Migrate from niri" step that sets
the engine to `niri.js`, asks for the user's `config.kdl`, and sets everything up from it.

### Each engine has its own config file

The main config holds bento's native settings. Each engine's binds and options live in a `.kdl`
file of their own, and bento loads the one for the running engine. Switching engines switches
which file is loaded, so nothing breaks, while validation stays strict against the running
engine.

The main config picks the engine with `tiling { layout "…" }`. A built-in engine's file is
`~/.config/bento/engines/<name>.kdl`, like `engines/niri.kdl`. An engine from a file has its
`.kdl` next to the `.js`, with the same name: `layout "~/layouts/mine.js"` loads
`~/layouts/mine.kdl`. The engine's file holds its `options {}` and its `binds {}`.

Config files never use `layout-action`: the engine's file names its actions directly, and the
main config can't bind them at all. `layout-action "name" args...` is only for running an action
of the running engine from the command line, `bento msg action layout-action ...`.

The built-in engines ship a default `.kdl` with their binds and options, and bento copies it to
`engines/<name>.kdl` the first time the engine runs without one, the way it writes the default
main config on first start. From then on the file is the user's, with no fallback to the
defaults. An engine loaded from a file ships its `.kdl` next to its `.js`.

bento's default main config binds only actions that work under every engine: spawning,
workspaces, monitors, screenshots, floating and the like. Binds to actions an engine declares,
niri's column actions included, are in that engine's default file.

All the files share one namespace: binding the same key in the main config and in the running
engine's file is an error, not one quietly winning over the other.

### bento declares its own actions

bento's native actions are declared like an engine's: one table gives each one its name, a
description and typed arguments, in the same shape as `meta.actions`. The Shortcuts page reads
it and the engine's declarations the same way, and the hotkey overlay takes its titles from it.

### Shortcuts can be edited in bento Settings

The Shortcuts page is for setting shortcuts, not only reading them. It shows one list of
binds, native and engine ones together, and doesn't show which file a bind lives in. Each bind is
a key with its action and the action's arguments, so `spawn` with different commands is just
several binds. Adding a shortcut picks the key, then the action and its arguments.

The app writes each change to the right file: a bind to an action the running engine declares
goes in the engine's file, and every other bind goes in the main config.

A bind's other settings, `repeat`, `cooldown-ms`, `allow-when-locked`, `allow-inhibiting` and
`hotkey-overlay-title`, can be changed in its edit dialog, under a collapsed "Advanced" section.
An edit keeps whatever it doesn't change, so settings written by hand survive.

While the page records a key, its window inhibits bento's shortcuts through the
keyboard-shortcuts-inhibit protocol, so bound keys reach it instead of running. Binds with
`allow-inhibiting=false` still go to bento.

Recording takes mouse and scroll triggers as well as keys: scrolling or clicking with modifiers
held records `WheelScroll…`, `TouchpadScroll…` or a mouse button, the way a key press records the
key.

A new shortcut can take a key that's already bound. The dialog says what the key does now and
offers to replace it, which removes the old bind from whichever file holds it. A key bound in an
included file can't be replaced, so the dialog says it's taken and where it comes from.

Restoring defaults works like GNOME's Keyboard Shortcuts: a changed shortcut can be reset on its
own, which puts its action back on its default key, and "Reset All" puts back the shipped binds
in the main config and the engine's file. An engine loaded from a file has no shipped defaults
apart from its own `.kdl`, so only bento's own shortcuts reset under it.

Binds from included files show on the page but can't be changed there, and their rows say which
file they come from. Those files often belong to something else, like a shell that generates
them: an edit in place would be lost when it rewrites the file, and an override in the main
config only wins when the `include` comes before its `binds {}`. The parsed config remembers
which file each bind comes from.

The page has a search bar that matches a bind's action, arguments and keys. The Engine page
doesn't list shortcuts again: it says how many actions the running engine has and how many have
a shortcut, and links to the Shortcuts page. Every action, bound or not, is in the add dialog's
action picker.

The page lists bento's actions and the running engine's. It can't see the actions of engines that
aren't running: Settings never runs engine code itself, and learns an engine's declarations only
from the running one. Switching engines on the Engine page brings up that engine's shortcuts.

## Plan

This work started from the Shortcuts page in bento Settings, the first of the pages still marked
as planned there. The order agreed on:

1. **Groundwork on today's engines**, so the Shortcuts page has something complete to read. Done, in
   #27:
   - `meta.actions` with typed arguments, checked when an engine loads and by `bento validate`;
   - each engine's options and binds in its own `.kdl` file, and default `.kdl` files for the
     built-in engines, written on first run.
2. **Engine action names, before the page.** Done, in #27. niri's layout actions are
   no longer bento's native actions: they're `niri.js`'s own, bound in its file. `bento.js` and
   the template have names that say what they do. bento's internal layout methods named after
   niri's actions are gone, with the inherited tests that only exercised niri's columns. bento has
   its own floating actions. Actions that cross a monitor or workspace edge, like
   `focus-column-or-monitor-left`, don't exist until the rework: engines can't see other monitors
   or workspaces before it, and bento adds no stopgap to fake it.

   The gaps this leaves until the rework, on purpose rather than faked:
   - no engine action crosses into another monitor or workspace;
   - `move-column-to-workspace` and `move-column-to-monitor` are gone with the rest of niri's
     names, and with them engines handing a group of windows to another workspace
     (`transferOut`/`transferIn`); bento's `move-window-to-*` move one window;
   - bento's own screens no longer borrow the engine's actions: the overview's left and right
     arrow keys and its horizontal wheel, the window switcher reusing focus-left and focus-right
     binds, and moving or resizing the screenshot selection with move and resize binds;
   - the hotkey overlay lists no engine actions;
   - double-clicking a window's resize edge no longer toggles its width or height;
   - the fallback engine, used when the configured one fails to load, has no actions;
   - the `default-window-height` window rule reaches the engine as niri's `set-window-height`, so
     only `niri.js` honours it, until engines declare their own window rules.
3. **The Shortcuts page**, as described above, with bento's own action table. Done, in #27.
4. **The engine rework**, everything else in this document: one engine for the session, the
   object model, events, drawn surfaces, grouping, the socket. This document is the reference
   for refactoring the repo.

The other planned pages in bento Settings (Displays, Workspaces, Window Rules, Gestures, Startup
Apps) and bento's own shell come after, in no agreed order yet.
