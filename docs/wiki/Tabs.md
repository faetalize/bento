### Overview

<sup>Since: 25.02</sup>

You can switch a column to present windows as tabs, rather than as vertical tiles.
All tabs in a column have the same window size, so this is useful to get more vertical space.

Both built-in [layout engines](./Configuration:-Tiling-Layouts.md) support tabs.
In the `niri` engine, a column shows its windows as tabs, as described on this page.
In the `bento` engine, windows become tabs of a tabbed area, which covers a rectangle of cells on the grid; see [Tabbed areas in the `bento` engine](#tabbed-areas-in-the-bento-engine) below.

![Terminal with a tab indicator on the left.](https://github.com/user-attachments/assets/0e94ac0d-796d-4f85-a264-c105ef41c13f)

Use this bind to toggle a column (or in the `bento` engine, a stack of windows) between normal and tabbed display:

```kdl
binds {
   Mod+W { toggle-column-tabbed-display; }
}
```

All other binds remain the same: switch tabs with `focus-window-down/up`, add or remove windows with `consume-window-into-column`/`expel-window-from-column`, and so on.

Unlike regular columns, tabbed columns can go full-screen with multiple windows.

### Tabbed areas in the `bento` engine

In the `bento` engine, a tabbed area covers a rectangle of cells, like a single window, and shows one of its windows at a time.
It moves and resizes as one.

- `toggle-column-tabbed-display` on a window gathers the windows stacked right above and below it, in exactly the same columns of cells, into one tabbed area covering all their rows.
On a tabbed area, it splits the tabs back into a stack, sharing the area's rows as evenly as possible.
If there are more tabs than rows, the extra windows go to the right of the stack.
`set-column-display` does the same, in one direction.
- `focus-window-down` and `focus-window-up` switch tabs, and go on to the next window on the grid past the last or the first tab.
- `move-window-down` and `move-window-up` reorder the tabs.
- [Consuming](./Grid-Layout.md#consume-and-expel) a window into a tabbed area adds it as the last tab, rather than splitting rows.
- `consume-or-expel-window-left` and `consume-or-expel-window-right` on a tab take it out of the area, and put it on that side of the area at the same size.
`expel-window-from-column` does the same to the right.
- When a tab closes, the area keeps its cells, so nothing else moves.
- A tabbed area goes fullscreen or maximized with all its tabs, and hides its tab indicator while it does.

With [`default-column-display "tabbed"`](./Configuration:-Layout.md#default-column-display), every new window opens as a tabbed area of its own.

### Tab indicator

Tabbed columns show a tab indicator on the side.
You can click on the indicator to switch tabs.

See the [`tab-indicator` section in the layout section](./Configuration:-Layout.md#tab-indicator) to configure it.

By default, the indicator draws "outside" the column (or tabbed area), so it can overlay other windows or go off-screen.
The `place-within-column` flag puts the indicator "inside" the column, adjusting the window size to make space for it.
This is especially useful for thicker tab indicators, or when you have very small gaps.

| Default | `place-within-column` |
| --- | --- |
| ![A screenshot showing 4 windows, with the middle column being focused. The tab indicator overflows onto the left column](https://github.com/user-attachments/assets/c2f51f50-3d87-403a-8beb-cbbe5ec5c880) | ![A screenshot showing 4 windows, with the middle column being focused. The tab indicator is contained within its respective column](https://github.com/user-attachments/assets/f1797cd0-d518-4be6-95b4-3540523c4370) |
