There are several ways to make a window big on bento: maximizing the column, maximizing the window to edges, and fullscreening the window.
Let's look at their differences.

## Maximized (full-width) columns

Maximizing the column via `maximize-column` (bound to <kbd>Mod</kbd><kbd>F</kbd> by default) expands its width to cover the whole screen.
Maximized columns still leave space for [struts] and [gaps], and can contain multiple windows.
The windows retain their borders.
This is the simplest of the sizing modes, and is equivalent to `proportion 1.0` column width, or `set-column-width "100%"`.

This describes the `niri` [layout engine](./Configuration:-Tiling-Layouts.md), which decides what `maximize-column` does.
In the `bento` engine, `maximize-column` toggles the focused window between the full width of the view and its previous width, keeping its rows; see [Grid Layout](./Grid-Layout.md#resizing-windows).

![Screenshot of a maximized column with two windows.](./img/maximized-column.png)

You can make a window open in a maximized column with the [`open-maximized true`](./Configuration:-Window-Rules.md#open-maximized) window rule.
In the `bento` engine, it opens as wide as the view.

## Windows maximized to edges

<sup>Since: 25.11</sup>

You can maximize an individual window via `maximize-window-to-edges` (bound to <kbd>Mod</kbd><kbd>M</kbd> by default).
This is the same maximize as you can find on other desktop environments and operating systems: it expands a window to the edges of the available screen area.
You will still see your bar, but not struts, gaps, or borders.

Windows are aware of their maximized-to-edges status and generally respond by squaring their corners.
Windows can also control maximizing-to-edges: when you click on the square icon in the window's titlebar, or double-click on the titlebar, the window will request bento to maximize or unmaximize itself.

You can put multiple maximized windows into a [tabbed column](./Tabs.md), but not into a regular column.
In the `bento` engine, a [tabbed area](./Tabs.md#tabbed-areas-in-the-bento-engine) is maximized with all its tabs.

![Screenshot of a window maximized to edges.](./img/window-maximized-to-edges.png)

You can make a window open maximized-to-edges, or prevent a window from maximizing upon opening, with the [`open-maximized-to-edges`](./Configuration:-Window-Rules.md#open-maximized-to-edges) window rule.

## Fullscreen windows

Windows can go fullscreen, usually seen with video players, presentations or games.
You can also force a window to go fullscreen via `fullscreen-window` (bound to <kbd>Mod</kbd><kbd>Shift</kbd><kbd>F</kbd> by default).
Fullscreen windows cover the entire screen.
Similarly to maximize-to-edges, windows are aware of their fullscreen status, and can respond by hiding their titlebars or other parts of the UI.

bento renders a solid black backdrop behind fullscreen windows.
This backdrop helps match the screen size when the window itself remains too small (e.g. if you try to fullscreen a fixed-size dialog window), which is the behavior [defined by the Wayland protocol](https://wayland.app/protocols/xdg-shell#xdg_toplevel:request:set_fullscreen).

When a fullscreen window is focused and not animating, it will cover floating windows and the top layer-shell layer.
If you want for example your layer-shell notifications or launcher to appear over fullscreen windows, configure the respective tools to put them on the overlay layer-shell layer.

![Screenshot of a fullscreen window.](./img/fullscreen-window.png)

You can make a window open fullscreen, or prevent a window from fullscreening upon opening, with the [`open-fullscreen`](./Configuration:-Window-Rules.md#open-fullscreen) window rule.

## Common behaviors across fullscreen and maximize

Fullscreen or maximized-to-edges windows can only be in the tiling layout.
So if you try to fullscreen or maximize a [floating window](./Floating-Windows.md), it'll move into the tiling layout.
Then, unfullscreening/unmaximizing will bring it back into the floating layout automatically.

Fullscreen and maximized windows remain a normal participant of the tiling layout, and the layout engine decides what happens around them.
With both built-in engines, thanks to scrollable tiling, you can scroll left and right from them and see other windows.
The `bento` engine gives a fullscreen or maximized-to-edges window enough cells to cover its width, across all rows, and pushes other windows aside; see [Grid Layout](./Grid-Layout.md#fullscreen-and-maximized-windows).

![Screenshot of the overview showing a fullscreen window with other windows side by side.](./img/fullscreen-window-in-overview.png)

Fullscreen and maximize-to-edges are both special states that the windows are aware of and can control.
Windows sometimes want to restore their fullscreen or, more frequently, maximized state when they open.
The best opportunity for this is during the *initial configure* sequence when the window tells bento everything it should know before opening the window.
If the window does this, then `open-maximized-to-edges` and `open-fullscreen` window rules have a chance to block or adjust the request.

However, some clients tend to request to be maximized shortly *after* the initial configure sequence, when bento already sent them the initial size request (sometimes even after showing on screen, resulting in a quick resize right after opening).
From bento's point of view, the window is already open by this point, so if the window does this, then the `open-maximized-to-edges` and `open-fullscreen` window rules don't do anything.

## Windowed fullscreen

<sup>Since: 25.05</sup>

bento can also tell a window that it's in fullscreen without actually making it fullscreen, via the `toggle-windowed-fullscreen` action.
This is generally useful for screencasting browser-based presentations, when you want to hide the browser UI, but still have the window sized as a normal window.

When in windowed fullscreen, you can use bento's actions to maximize or unmaximize the window.
Window-side titlebar maximize buttons and gestures may not work, since the window will always think that it's in fullscreen.

See also windowed fullscreen on the [screencasting features wiki page](./Screencasting.md#windowed-fakedetached-fullscreen).


[struts]: ./Configuration:-Layout.md#struts
[gaps]: ./Configuration:-Layout.md#gaps
