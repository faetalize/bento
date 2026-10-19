### Overview

<sup>Since: 25.02</sup>

You can gather windows stacked on top of each other into one area that presents them as tabs.
All tabs in an area have the same window size, so this is useful to get more vertical space.

![Terminal with a tab indicator on the left.](https://github.com/user-attachments/assets/0e94ac0d-796d-4f85-a264-c105ef41c13f)

Use this bind to toggle between normal and tabbed display:

```kdl
binds {
   Mod+W { toggle-column-tabbed-display; }
}
```

On bento's [grid](./Grid-Layout.md), `toggle-column-tabbed-display` (and `set-column-display`) gathers the focused window and the windows stacked right above and below it in exactly the same columns of cells into one tabbed area.
Toggling again splits the tabs back into a stack of windows sharing the area's rows.

Other binds work on tabs like this:

- `focus-window-down` and `focus-window-up` switch tabs before leaving the area.
- `move-window-down` and `move-window-up` reorder tabs.
- Consuming a window into a tabbed area, for example with `consume-window-into-column` or `consume-or-expel-window-left`, adds it as a tab.
- `consume-or-expel-window-left/right` or `expel-window-from-column` on a tab takes it out of the area and puts it beside the area.

Set [`default-column-display "tabbed"`](./Configuration:-Layout.md#default-column-display) to make new windows open as a tabbed area with a single tab.

Unlike stacked windows, a tabbed area can go full-screen with multiple windows.

> [!NOTE]
> Dragging a window into a tabbed area with the mouse to add a tab is not supported yet.

### Tab indicator

Tabbed areas show a tab indicator on the side.
You can click on the indicator to switch tabs.

See the [`tab-indicator` section in the layout section](./Configuration:-Layout.md#tab-indicator) to configure it.

By default, the indicator draws "outside" the tabbed area, so it can overlay other windows or go off-screen.
The `place-within-column` flag puts the indicator "inside" the area, adjusting the window size to make space for it.
This is especially useful for thicker tab indicators, or when you have very small gaps.

| Default | `place-within-column` |
| --- | --- |
| ![A screenshot showing 4 windows, with the middle column being focused. The tab indicator overflows onto the left column](https://github.com/user-attachments/assets/c2f51f50-3d87-403a-8beb-cbbe5ec5c880) | ![A screenshot showing 4 windows, with the middle column being focused. The tab indicator is contained within its respective column](https://github.com/user-attachments/assets/f1797cd0-d518-4be6-95b4-3540523c4370) |
