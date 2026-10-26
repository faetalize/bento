### Overview

<sup>Since: 25.01</sup>

Floating windows in bento always show on top of the tiled windows.
The floating layout does not scroll.
Each workspace/monitor has its own floating layout, just like each workspace/monitor has its own tiling layout.

Floating windows are bento's own: the [layout engine](./Configuration:-Tiling-Layouts.md) only arranges tiled windows, so floating works the same whichever engine you use.

New windows will automatically float if they have a parent (e.g. dialogs) or if they are fixed size (e.g. splash screens).
To change a window between floating and tiling, you can use the `toggle-window-floating` bind or right click while dragging/moving the window.
You can also use the `open-floating true/false` window rule to either force a window to open as floating, or to disable the automatic floating logic.

When a window goes back to tiling, the layout engine decides where it goes.
The `bento` engine puts it on the cells under where it was floating, at the size it had when it was last tiled.

Use `switch-focus-between-floating-and-tiling` to switch the focus between the two layouts.
When focused on the floating layout, binds (like `focus-column-right`) will operate on the floating window, rather than going to the layout engine.

You can precisely position a floating window with a command like `bento msg action move-floating-window -x 100 -y 200`.
