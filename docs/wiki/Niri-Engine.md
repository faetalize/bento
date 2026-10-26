### Overview

The `niri` engine is niri's scrollable tiling as a bento [layout engine](./Configuration:-Tiling-Layouts.md).
Windows live in columns on a strip that scrolls horizontally without end.
A column holds one or more windows, stacked or shown as [tabs](./Tabs.md).
Opening a window adds a column next to the focused one, and the view scrolls to keep the focused column in view.

Pick it in the `tiling` section:

```kdl
tiling {
    layout "niri"
}
```

The engine is a port of niri's scrolling layout, [`src/layout/scrolling.rs`](https://github.com/niri-wm/niri/blob/main/src/layout/scrolling.rs) in upstream niri, to JavaScript: [`resources/layouts/niri.js`](https://github.com/faetalize/bento/blob/main/resources/layouts/niri.js).
Its functions keep niri's names where they do the same thing, so the two are easy to compare.

### What works like in niri

- niri's layout actions, like `focus-column-left`, `consume-or-expel-window-left` or `set-column-width`, do what they do in niri.
  Actions that combine others, like `focus-column-or-monitor-left` or `focus-window-or-workspace-down`, do their part within the workspace, and bento does the rest when nothing changes, as niri does.
- The [`layout {}`](./Configuration:-Layout.md) options that describe columns: `default-column-width`, `preset-column-widths`, `preset-window-heights`, `center-focused-column`, `always-center-single-column`, `default-column-display`, `gaps` and `struts`.
- Window rules that size and place new windows, like [`default-column-width`](./Configuration:-Window-Rules.md#default-column-width), `default-window-height` and `open-maximized`.
- Tabbed columns, fullscreen and maximized windows, resizing with the mouse, dragging windows between columns, and swipes that snap the view to columns.
- The IPC: a window's `pos_in_scrolling_layout` is its column and its position in the column, like in niri.

### What's different

- **Animations are bento's.**
  bento animates the windows and the view the same way for every engine, with the [`window-movement`](./Configuration:-Animations.md) and `horizontal-view-movement` animations, so they can look slightly different from niri's own.
- **No actions of its own.**
  The engine has no `layout-action`s; everything goes through niri's actions.
- **Switching engines starts from where the windows are.**
  The engine keeps its columns when bento reloads it, but when you switch to it from another engine, it builds columns from the windows' positions.

Everything outside the tiled windows, like workspaces, monitors, floating windows and the overview, is bento's, the same as with every engine.
