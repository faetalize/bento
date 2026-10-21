### Overview

The `bento` layout, bento's default [tiling layout](./Configuration:-Tiling-Layouts.md), puts tiled windows on a grid of cells.
Like niri's strip of columns, the grid scrolls horizontally without end, and every workspace has its own grid.
With [left gravity](#left-gravity), the grid starts at a fixed left edge that windows can't be pushed past; without it, the grid goes on to the left too.
Unlike niri, a window doesn't have to fill a column from top to bottom.
Every window covers a rectangle of whole cells: a starting column and row, plus a span.
So windows can sit in different rows, stack on top of each other, and span several cells in either direction.

The grid has 12 rows.
Cells are close to square: bento works out an even number of columns across the view from the output size and the [gaps](./Configuration:-Layout.md#gaps).
Gaps go between cells.

> [!NOTE]
> bento keeps niri's action and option names, so many of them still say "column", like `move-column-left` or `default-column-width`.
> See [Columns and stacks](#columns-and-stacks) for what a column means on the grid.

### Sizes

Window sizes snap to whole cells.

- `proportion` sizes round down to whole cells.
Complementary proportions like <sup>1</sup>&frasl;<sub>3</sub> and <sup>2</sup>&frasl;<sub>3</sub> fit together, but may leave a cell free.
For example, if the view is 16 cells wide, <sup>1</sup>&frasl;<sub>3</sub> is 5 cells and <sup>2</sup>&frasl;<sub>3</sub> is 10 cells, leaving one cell free.
- `fixed` sizes in logical pixels go to the nearest number of cells.
Like in niri, a fixed width or height is for the window itself, without its border.

A window remembers its size on the grid.
It keeps that size when you move it to another workspace or monitor, when you make it floating and tile it again, and when you toggle full width.

When the number of cells across the view changes, for example on an output of a different size, bento works the sizes out again so that proportional sizes stay proportional.

### Left gravity

By default, windows slide left within their rows until they touch another window.
This closes the gaps left by closed, shrunk or moved windows, much like columns close up in niri.
Holes remain only where nothing fits.

You can turn this off with the layout's `gravity` option:

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

### Opening windows

New windows open to the right of the focused window, lined up with its top row.
Windows that are in the way get pushed to the right.

New windows get their size from [`default-column-width`](./Configuration:-Layout.md#default-column-width) and the [`default-window-height`](./Configuration:-Window-Rules.md#default-window-height) window rule, snapped to cells.
The [`default-column-width`](./Configuration:-Window-Rules.md#default-column-width) window rule works too.

### Moving windows

- `move-column-left` and `move-column-right` move the focused window one cell to the side, swapping places with a window it runs into.
- `move-window-up` and `move-window-down` move it one cell up or down.
- `move-column-to-first` and `move-column-to-last` move it to the start of the strip, or past its end, staying in its rows.
- `move-column-to-index` uses the [column numbering](#columns-and-stacks) below.

When you drag a window with the mouse, bento shows the cells where it will land, after pushing other windows aside and applying left gravity, and drops it there.

Holding the pointer close to an edge of another window splits that window instead: the dragged window takes the half of its cells on that side, and the other window keeps the rest.
The edges are a cell deep, or a quarter of the window for small windows, so dropping in the middle of a window works as usual.
A window that is one cell wide or tall can't be split that way.

### Resizing windows

`set-column-width`, `set-window-height`, `switch-preset-column-width`, `switch-preset-window-height` and their `-back` variants all work in cells.
Adjustments like `"+10%"` or `"-100"` change the size by the nearest number of cells, and always by at least one cell.

`maximize-column` toggles the window between full width and its previous width.

`expand-column-to-available-width` grows the window to the right, then to the left, into empty cells in its rows, up to the nearest window or the edge of the view.
It doesn't push anything aside.

When you drag a window edge with the mouse, the size snaps to cells and neighbouring windows get pushed aside as you go.
The edge opposite the one you're dragging stays still.
With left gravity, windows can't be pushed past the left edge of the grid, and they're always packed against it, so dragging a left edge can only shrink the window.
Without gravity, dragging a left edge grows the window to the left too, pushing the windows there further left.
The window you're resizing stays put until you let go, then falls into place with left gravity.

Resizing a window's height keeps stacks together.
When a window is stacked right on top of another one in exactly the same columns, the edge they share moves for both: growing one takes rows from the other, and shrinking gives them back.
The edge stops where the other window can't get any smaller.
A window at the bottom of the grid grows upwards, taking rows from the window above it the same way.
This works from the keyboard and when dragging a top or bottom edge with the mouse.
Windows that only partly overlap the edge get pushed instead.

### Splitting and swapping

The bento layout has three actions of its own for two windows at a time:

- `layout-action "split-h"` splits a window's cells in two side by side, and gives one half to another window.
- `layout-action "split-v"` does the same with stacked halves.
- `layout-action "swap"` swaps two windows' places and sizes.

Each takes two presses.
The first press picks the focused window and highlights it.
Then focus another window as usual, and press again.
For a split, the second window moves into the half of the first one on its side, judged by their centers; when they're level, it takes the right or bottom half.
The first window keeps the larger half of an odd number of cells, and windows one cell wide (or tall, for `split-v`) can't be split that way.
Pressing again on the window you picked first cancels.

For example, with A and B stacked on the left and C beside them, focusing A, pressing `split-h`, focusing C and pressing `split-h` again puts C next to A, above B.

The cells the second window leaves free up, and with left gravity the windows around them slide in.
So a window that moves into cells to the right of where it was can slide back into its old place if nothing else takes it.

`layout-action "stuff"` puts the focused window between two windows that touch, side by side or stacked.
The first press highlights where the window would go, at the seam nearest to it.
While the highlight is up, `focus-column-left`, `focus-column-right`, `focus-window-up` and `focus-window-down` move it to the next seam in that direction, and any other action cancels.
The second press puts the window there.

The two windows keep their outer edges and make room: their span is split in three, and the window takes the middle third, so two windows 11 cells wide become 7, 8 and 7.
Across the seam, the window only takes the rows (or columns) that the two windows share; if one is taller, it shrinks at its full height.
Only seams in view count.

### Columns and stacks

Some actions talk about columns.
On the grid, "column" means one of two things.

The grid's columns are the places where windows start, numbered from left to right starting at 1.
This numbering is used by `focus-column N`, `move-column-to-index N`, and the `pos_in_scrolling_layout` field in the [IPC](./IPC.md).

For actions that work within a column, like `focus-window-top`, `focus-window-bottom` and `focus-window-in-column N`, a window's column is its stack: the windows covering the same column of cells, from top to bottom.
Windows in a [tabbed area](./Tabs.md) count in tab order.

### Focus

Directional focus remembers the line you're travelling along: the row when going left and right, and the column of cells when going up and down.
This way, going past a big window brings you back to the same line on the other side.

- `focus-window-down` and `focus-window-up` can reach the nearest window further down or up, even if it's diagonal.
- Actions that fall back to something else, like `focus-window-or-workspace-down`, `focus-window-or-monitor-up` or `focus-window-down-or-column-left`, only count windows directly below or above that share columns of cells.
- `focus-column-first` and `focus-column-last` go to the leftmost or rightmost window in the row you're travelling along.
The wrap-arounds `focus-column-right-or-first` and `focus-column-left-or-last` do the same.
- `focus-window-top`, `focus-window-bottom` and `focus-window-in-column` work within the stack, so `focus-window-down-or-top` and `focus-window-up-or-bottom` wrap around within it.

When you close the focused window, focus goes back to the window it was opened from, if focus didn't change in between.
The view doesn't scroll back to where it was, unless that window is out of view.
Otherwise, focus goes to the window below, then above, then right, then left of it.

### The view

Layout changes don't move the view.
When windows open, close, move or resize, the ones that move slide into place on screen, while the rest stay still.
The view only scrolls to keep the focused window in view, and for actions about the view itself, like focusing another window, swiping, centering, or going fullscreen.

[`center-focused-column`](./Configuration:-Layout.md#center-focused-column) `"always"` and `"on-overflow"`, and the `center-visible-columns` action, work with the windows' spans on the grid.
[`always-center-single-column`](./Configuration:-Layout.md#always-center-single-column) applies when no two windows are side by side.
Windows stacked on top of each other count as a single column.

On a touchpad, swiping the view snaps to window edges on the grid.
After the swipe, the furthest fully visible window in the swipe direction gets focus, preferring the row you were in.

### Fullscreen and maximized windows

[Fullscreen and maximized-to-edges windows](./Fullscreen-and-Maximize.md) take enough cells to cover their width, across all rows.
Other windows get pushed aside, so nothing draws over them.
When a window stops being fullscreen or maximized, it gets its own size and row back.

### Consume and expel

- `consume-or-expel-window-left` and `consume-or-expel-window-right` on a window that isn't stacked put it into the stack beside it.
The window beside it keeps the top half of its rows, and the consumed window takes the bottom half.
- On a stacked window, they expel it instead.
It goes full height to the left or right of everything sharing its columns of cells, and the window it was split from grows back over its rows.
- `consume-window-into-column` brings the window on the right under the focused window.
- `expel-window-from-column` expels the focused window to the right.

See the [Tabs](./Tabs.md) page for how consume and expel work with tabbed areas.

### IPC

bento keeps niri's IPC and `NIRI_SOCKET`, so `niri msg` and other tools keep working.

A window's `pos_in_scrolling_layout` is (column, position in the stack), both starting at 1.
It uses the [column numbering and stacks](#columns-and-stacks) above, so it matches `focus-column` and `focus-window-in-column`.
