// niri's scrollable tiling, as a bento layout engine.
//
// Windows live in columns on an infinite horizontal strip. A column holds one or more tiles,
// stacked vertically or shown as tabs. Opening a window adds a column next to the focused one, and
// the camera scrolls to keep the focused column in view.
//
// The engine runs once for the whole session and keeps a strip for each workspace. Actions that
// cross a strip's edge, like focusing the monitor to the left, or moving a column to another
// workspace, are handled between strips by `create`. Tabbed columns are bento groups, and the tab
// indicator and the insert hint are drawn with nodes.
//
// This is a port of niri's `src/layout/scrolling.rs`. Functions keep niri's names where they do the
// same thing, so the two are easy to compare. bento animates windows and the camera, so everything
// niri does for animations is left out, and windows take the size they're given right away, so the
// end of every call works like niri getting a commit from every window that changed.
//
// Coordinates: column `i` starts at canvas x `columnX(i)`, like niri's `column_x`, and the camera
// is at `columnX(activeColumnIdx) + viewOffset`. The camera's y stays 0, so canvas y is view y.

// Arguments shared by several actions.
const INDEX = [{ name: "index", type: "number", description: "Counting from 1." }];
const SIZE = [
    {
        name: "change",
        type: "size",
        description: 'A size like "50%" or "800", or a change like "+10%" or "-100".',
    },
];
// Whether focus goes along with what moved, like niri's `focus=false`.
const FOCUS = {
    name: "focus",
    type: "boolean",
    default: true,
    description: "Whether focus follows what moves.",
};

export const meta = {
    api: 2,
    name: "niri",
    description: "niri's scrollable tiling: columns of windows on an infinite horizontal strip.",
    // niri's layout actions, all of which this engine handles like niri does. Each workspace has
    // its own strip, and the actions that cross a strip's edge go to the next monitor or workspace.
    actions: {
        "focus-column-left": { description: "Focus the column to the left." },
        "focus-column-right": { description: "Focus the column to the right." },
        "focus-column-first": { description: "Focus the first column." },
        "focus-column-last": { description: "Focus the last column." },
        "focus-column-right-or-first": {
            description: "Focus the column to the right, or the first one from the last one.",
        },
        "focus-column-left-or-last": {
            description: "Focus the column to the left, or the last one from the first one.",
        },
        "focus-column": { description: "Focus a column by its index.", args: INDEX },
        "focus-column-or-monitor-left": {
            description: "Focus the column to the left, or the monitor to the left from the first one.",
        },
        "focus-column-or-monitor-right": {
            description: "Focus the column to the right, or the monitor to the right from the last one.",
        },
        "focus-window-in-column": {
            description: "Focus a window in the focused column by its index.",
            args: INDEX,
        },
        "focus-window-down": { description: "Focus the window below in the column." },
        "focus-window-up": { description: "Focus the window above in the column." },
        "focus-window-or-monitor-down": {
            description: "Focus the window below, or the monitor below from the bottom window.",
        },
        "focus-window-or-monitor-up": {
            description: "Focus the window above, or the monitor above from the top window.",
        },
        "focus-window-or-workspace-down": {
            description: "Focus the window below, or the workspace below from the bottom window.",
        },
        "focus-window-or-workspace-up": {
            description: "Focus the window above, or the workspace above from the top window.",
        },
        "focus-window-down-or-column-left": {
            description: "Focus the window below, or the column to the left from the bottom window.",
        },
        "focus-window-down-or-column-right": {
            description: "Focus the window below, or the column to the right from the bottom window.",
        },
        "focus-window-up-or-column-left": {
            description: "Focus the window above, or the column to the left from the top window.",
        },
        "focus-window-up-or-column-right": {
            description: "Focus the window above, or the column to the right from the top window.",
        },
        "focus-window-top": { description: "Focus the top window in the column." },
        "focus-window-bottom": { description: "Focus the bottom window in the column." },
        "focus-window-down-or-top": {
            description: "Focus the window below, or the top window from the bottom one.",
        },
        "focus-window-up-or-bottom": {
            description: "Focus the window above, or the bottom window from the top one.",
        },
        "move-column-left": { description: "Move the focused column to the left." },
        "move-column-right": { description: "Move the focused column to the right." },
        "move-column-left-or-to-monitor-left": {
            description: "Move the focused column to the left, or to the monitor to the left from the first place.",
            args: [FOCUS],
        },
        "move-column-right-or-to-monitor-right": {
            description: "Move the focused column to the right, or to the monitor to the right from the last place.",
            args: [FOCUS],
        },
        "move-column-to-first": { description: "Move the focused column to the start." },
        "move-column-to-last": { description: "Move the focused column to the end." },
        "move-column-to-index": {
            description: "Move the focused column to an index.",
            args: INDEX,
        },
        "move-column-to-workspace": {
            description: "Move the focused column to a workspace of the focused monitor.",
            args: [
                {
                    name: "workspace",
                    type: "workspace",
                    description: "An index counting from 1, or a name.",
                },
                FOCUS,
            ],
        },
        "move-column-to-workspace-down": {
            description: "Move the focused column to the workspace below.",
            args: [FOCUS],
        },
        "move-column-to-workspace-up": {
            description: "Move the focused column to the workspace above.",
            args: [FOCUS],
        },
        "move-column-to-monitor": {
            description: "Move the focused column to a monitor's active workspace.",
            args: [{ name: "monitor", type: "monitor", description: "A connector name, like DP-2." }, FOCUS],
        },
        "move-column-to-monitor-left": {
            description: "Move the focused column to the monitor to the left.",
            args: [FOCUS],
        },
        "move-column-to-monitor-right": {
            description: "Move the focused column to the monitor to the right.",
            args: [FOCUS],
        },
        "move-column-to-monitor-up": {
            description: "Move the focused column to the monitor above.",
            args: [FOCUS],
        },
        "move-column-to-monitor-down": {
            description: "Move the focused column to the monitor below.",
            args: [FOCUS],
        },
        "move-column-to-monitor-previous": {
            description: "Move the focused column to the previous monitor.",
            args: [FOCUS],
        },
        "move-column-to-monitor-next": {
            description: "Move the focused column to the next monitor.",
            args: [FOCUS],
        },
        "move-window-down": { description: "Move the focused window down in its column." },
        "move-window-up": { description: "Move the focused window up in its column." },
        "move-window-down-or-to-workspace-down": {
            description: "Move the focused window down in its column, or to the workspace below from the bottom.",
            args: [FOCUS],
        },
        "move-window-up-or-to-workspace-up": {
            description: "Move the focused window up in its column, or to the workspace above from the top.",
            args: [FOCUS],
        },
        "consume-or-expel-window-left": {
            description: "Put the focused window into the column to the left, or take it out of its column to the left.",
        },
        "consume-or-expel-window-right": {
            description: "Put the focused window into the column to the right, or take it out of its column to the right.",
        },
        "consume-window-into-column": {
            description: "Put the first window of the column to the right into the focused column.",
        },
        "expel-window-from-column": {
            description: "Take the bottom window of the focused column out into a column of its own.",
        },
        "swap-window-left": {
            description: "Swap the focused window with the one in the column to the left.",
        },
        "swap-window-right": {
            description: "Swap the focused window with the one in the column to the right.",
        },
        "toggle-column-tabbed-display": {
            description: "Switch the focused column between tabs and stacked windows.",
        },
        "set-column-display": {
            description: "Show the focused column's windows as tabs or stacked.",
            args: [{ name: "display", type: "choice", choices: ["normal", "tabbed"] }],
        },
        "center-column": { description: "Center the focused column in the view." },
        "center-window": { description: "Center the window's column, if it's the focused one." },
        "center-visible-columns": {
            description: "Center the columns that are fully in view, together.",
        },
        "switch-preset-column-width": {
            description: "Switch the focused column to the next preset width.",
        },
        "switch-preset-column-width-back": {
            description: "Switch the focused column to the previous preset width.",
        },
        "switch-preset-window-width": {
            description: "Switch the window's column to the next preset width.",
        },
        "switch-preset-window-width-back": {
            description: "Switch the window's column to the previous preset width.",
        },
        "switch-preset-window-height": {
            description: "Switch the window to the next preset height.",
        },
        "switch-preset-window-height-back": {
            description: "Switch the window to the previous preset height.",
        },
        "maximize-column": {
            description: "Toggle the focused column between the full view width and its own width.",
        },
        "set-column-width": {
            description: "Set or change the focused column's width.",
            args: SIZE,
        },
        "set-window-width": { description: "Set or change the window's column width.", args: SIZE },
        "set-window-height": { description: "Set or change the window's height.", args: SIZE },
        "reset-window-height": {
            description: "Give the window back its automatic height in its column.",
        },
        "expand-column-to-available-width": {
            description: "Widen the focused column into the space other fully visible columns leave.",
        },
    },
    options: {
        "center-focused-column": {
            type: "choice",
            choices: ["never", "always", "on-overflow"],
            default: "never",
            description: "When to center a column that gets focus.",
        },
        "always-center-single-column": {
            type: "boolean",
            default: false,
            description: "Center a column that is alone on its workspace.",
        },
        "default-column-width": {
            type: "size",
            default: "50%",
            description: "The width new columns open with. Left empty, the window picks.",
        },
        "preset-column-widths": {
            type: "size",
            list: true,
            default: ["33.333%", "50%", "66.667%"],
            description: "The widths that switching preset column widths goes through.",
        },
        "preset-window-heights": {
            type: "size",
            list: true,
            default: ["33.333%", "50%", "66.667%"],
            description: "The heights that switching preset window heights goes through.",
        },
        "default-column-display": {
            type: "choice",
            choices: ["normal", "tabbed"],
            default: "normal",
            description: "How a new column shows its windows.",
        },
        "tab-indicator": {
            type: "boolean",
            default: true,
            description: "Draw an indicator along tabbed columns.",
        },
        "tab-indicator-position": {
            type: "choice",
            choices: ["left", "right", "top", "bottom"],
            default: "left",
            description: "The side of the column the tab indicator is on.",
        },
        "tab-indicator-width": {
            type: "number",
            default: 4,
            description: "How thick the tab indicator is.",
        },
        "tab-indicator-gap": {
            type: "number",
            default: 5,
            description: "The space between the tab indicator and the column; negative to overlap.",
        },
        "tab-indicator-length": {
            type: "number",
            default: 0.5,
            description: "How much of the column's side the tab indicator takes, as a proportion.",
        },
        "tab-indicator-corner-radius": {
            type: "number",
            default: 0,
            description: "The corner radius of each tab of the indicator.",
        },
        "tab-indicator-place-within-column": {
            type: "boolean",
            default: false,
            description: "Draw the tab indicator inside the column, which leaves room for it.",
        },
        "tab-indicator-hide-when-single-tab": {
            type: "boolean",
            default: false,
            description: "Leave out the tab indicator while a column has one tab.",
        },
        "tab-indicator-active-color": {
            type: "string",
            default: "#7fc8ff",
            description: "The color of the active tab.",
        },
        "tab-indicator-inactive-color": {
            type: "string",
            default: "#505050",
            description: "The color of the other tabs.",
        },
        "tab-indicator-urgent-color": {
            type: "string",
            default: "#9b0000",
            description: "The color of a tab whose window is urgent.",
        },
        "insert-hint": {
            type: "boolean",
            default: true,
            description: "Show where a dragged window would land.",
        },
        "insert-hint-color": {
            type: "string",
            default: "#7fc8ff80",
            description: "The color of the insert hint.",
        },
    },
    windowRules: {
        "default-column-width": {
            type: "size",
            default: null,
            description: "The width the window's column opens with. Left empty, the window picks.",
        },
        "default-window-height": {
            type: "size",
            default: null,
            description: "The height the window opens with in its column.",
        },
        "default-column-display": {
            type: "choice",
            choices: ["normal", "tabbed"],
            default: "normal",
            description: "How the window's new column shows its windows.",
        },
        "open-maximized": {
            type: "boolean",
            default: false,
            description: "Open the window as a column as wide as the view.",
        },
    },
};

/** Space between the tabs of a tab indicator, in logical pixels. */
const TAB_SPACING = 2;
/** Corner radius of the insert hint. */
const HINT_CORNER_RADIUS = 8;
/** How much of its speed a touchpad swipe keeps each millisecond, niri's `DECELERATION_TOUCHPAD`. */
const SWIPE_DECELERATION = 0.997;

// Limits from niri, which keep sizes in a range where nothing overflows.
const MAX_PX = 100000;
const MAX_F = 10000;

const clamp = (value, min, max) => Math.min(Math.max(value, min), max);

/** Applies a window's min and max size, where 0 means no limit. */
function applyLimits(value, min, max) {
    if (max > 0) value = Math.min(value, max);
    if (min > 0) value = Math.max(value, min);
    return value;
}

const sameSize = (a, b) => a !== null && b !== null && a.w === b.w && a.h === b.h;
const sizeKey = (size) => `${size.w}x${size.h}`;
const samePresets = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/** Index of the smallest `key(item)`, the first one on ties. */
function argmin(items, key) {
    let best = 0;
    for (let i = 1; i < items.length; i++) {
        if (key(items[i]) < key(items[best])) best = i;
    }
    return best;
}

/** niri's `SizeChange` for a preset, as `set-column-width` would get it. */
function sizeChangeFromPreset(preset) {
    return "proportion" in preset
        ? { op: "set", value: preset.proportion * 100, unit: "%" }
        : { op: "set", value: preset.fixed, unit: "px" };
}

/** niri's `ColumnWidth::from(PresetSize)`. */
function columnWidthFromPreset(preset) {
    return "proportion" in preset
        ? { proportion: clamp(preset.proportion, 0, MAX_F) }
        : { fixed: clamp(preset.fixed, 1, MAX_PX) };
}

function sameWidth(a, b) {
    return "proportion" in a ? a.proportion === b.proportion : a.fixed === b.fixed;
}

/**
 * View offset that shows a column at `newColX` with as little movement from a view at `curX` as
 * possible, niri's `compute_new_view_offset`. Relative to the column's x.
 */
function computeNewViewOffset(curX, viewWidth, newColX, newColWidth, gaps) {
    // If the column is wider than the view, always left-align it.
    if (viewWidth <= newColWidth) return 0;

    // The padding is smaller for columns that almost fill the view.
    const padding = clamp((viewWidth - newColWidth) / 2, 0, gaps);
    const newX = newColX - padding;
    const newRightX = newColX + newColWidth + padding;

    // If the column is already fully visible, leave the view as is.
    if (curX <= newX && newRightX <= curX + viewWidth) return -(newColX - curX);

    // Otherwise, prefer the alignment that results in less motion from the current position.
    const distToLeft = Math.abs(curX - newX);
    const distToRight = Math.abs(curX + viewWidth - newRightX);
    return distToLeft <= distToRight ? -padding : -(viewWidth - padding - newColWidth);
}

/**
 * The columns of one workspace. The engine keeps one for every workspace, and each only knows its
 * own: moving a column to another workspace takes it out of one and puts it in another.
 */
function createStrip(bento, ws) {
    // Engine state. `save()` writes all of it out, and `adopt()` reads it back.

    /** Columns from left to right. */
    let columns = [];
    let activeColumnIdx = 0;
    /** Where the camera is relative to the active column's x. */
    let viewOffset = 0;
    /**
     * Whether to activate the previous, rather than the next, column when the active column goes
     * away, and the view offset to restore for it.
     *
     * When a new column is opened and closed with no focus changes in between, it's more natural
     * to go back to the column that was focused before. New columns only open right of the active
     * one, so the previous column is always the one to the left.
     */
    let activatePrevColumnOnRemoval = null;
    /** View offset to restore when the active column leaves fullscreen or maximized. */
    let viewOffsetToRestore = null;
    /** An ongoing pointer resize: `{ win, edges, originalWindowSize }`. */
    let interactiveResize = null;
    /** The column width of the window being dragged, to give back when it's dropped. */
    let dragged = null;
    let lastOptions = bento.options;
    /** What `ws.focused` was at the end of the last call. */
    let lastFocused = null;
    let nextColumnId = 1;
    /** Whether a swipe is moving the camera, which the strip leaves alone until it ends. */
    let swiping = false;
    /** The nodes that draw each tabbed column's tab indicator, by column id. */
    const tabNodes = new Map();
    /** The node that shows where a dragged window would land, while one is over the workspace. */
    let hintNode = null;
    /** What the strip told the outside about itself last, see `exposed()`. */
    let lastExposed = null;

    // Per call: set while a call runs, and handled at its end.
    let skipCameraAnimation = false;
    let refitView = false;
    let viewSettled = false;
    /** Speed to continue into the camera's animation, in px/s, while a swipe ends. */
    let cameraVelocity = 0;

    // Geometry.

    const options = (name) => bento.options[name];
    const gaps = () => ws.config.gaps;
    const viewSize = () => ws.monitor.size;
    const round = (value) => ws.monitor.round(value);
    /** The area that maximized windows take: the view minus panels. */
    const parentArea = () => ws.monitor.workingArea;

    /** The area that columns fit in: the parent area minus struts, starting at a physical pixel. */
    const workingArea = () => ws.tilingArea;

    // Window sizes in this API are tile sizes, borders included. niri's fixed sizes (like
    // `set-window-height 500` or a `fixed` preset) are for the window inside the borders.
    const tileWidthForWindowWidth = (tile, w) => w + tile.win.borders.w;
    const tileHeightForWindowHeight = (tile, h) => h + tile.win.borders.h;
    const windowWidthForTileWidth = (tile, w) => w - tile.win.borders.w;
    const windowHeightForTileHeight = (tile, h) => h - tile.win.borders.h;

    // Window rules. A rule that no rule sets is left out of `rules`, so the option of the same name
    // is the default, if there is one. `null` is a size left to the window.

    const rulesWidth = (rules) =>
        rules["default-column-width"] !== undefined
            ? rules["default-column-width"]
            : options("default-column-width");
    const rulesHeight = (rules) => rules["default-window-height"] ?? null;
    const rulesDisplay = (rules) =>
        rules["default-column-display"] ?? options("default-column-display");

    /** Smallest tile size, at least 1 px of window. */
    function minSize(tile) {
        const { minSize: min, borders } = tile.win;
        return { w: Math.max(min.w, borders.w + 1), h: Math.max(min.h, borders.h + 1) };
    }

    /**
     * niri's `resolve_preset_size`: proportions give a tile size, fixed sizes a window size.
     * `extra` is space that the tab indicator takes.
     */
    function resolvePresetSize(preset, length, extra = 0) {
        if ("proportion" in preset) {
            return { tile: (length - gaps()) * preset.proportion - gaps() - extra };
        }
        return { window: preset.fixed };
    }

    function resolveColumnWidth(col, width) {
        if ("proportion" in width) {
            return (workingArea().w - gaps()) * width.proportion - gaps() - extraSize(col).w;
        }
        return width.fixed;
    }

    // The tab indicator. The strip draws it, and tabbed columns leave room for it when it's placed
    // within the column.

    /**
     * Space the indicator takes inside a tabbed area of `count` tabs: `w` and `h` to leave free
     * across and down, and `x` and `y` for how far the tabs start from the area's left and top
     * edges. All 0 when the indicator goes outside the area or is hidden, which it does by default.
     */
    function tabIndicatorSpace(count) {
        const none = { x: 0, y: 0, w: 0, h: 0 };
        if (
            !options("tab-indicator") ||
            !options("tab-indicator-place-within-column") ||
            (options("tab-indicator-hide-when-single-tab") && count === 1)
        ) {
            return none;
        }
        const size = Math.max(
            0,
            round(options("tab-indicator-width")) + round(options("tab-indicator-gap")),
        );
        switch (options("tab-indicator-position")) {
            case "left":
                return { x: size, y: 0, w: size, h: 0 };
            case "right":
                return { x: 0, y: 0, w: size, h: 0 };
            case "top":
                return { x: 0, y: size, w: 0, h: size };
            default:
                return { x: 0, y: 0, w: 0, h: size };
        }
    }

    /** Space the indicator takes in a column, niri's `TabIndicator::extra_size`. */
    function extraSize(col) {
        return col.display === "tabbed"
            ? tabIndicatorSpace(col.tiles.length)
            : { x: 0, y: 0, w: 0, h: 0 };
    }

    /** Extra size for a new column with one window. */
    function newColumnExtraSize() {
        return options("default-column-display") === "tabbed"
            ? tabIndicatorSpace(1)
            : { x: 0, y: 0, w: 0, h: 0 };
    }

    /** Where the tiles start because of the indicator, niri's `TabIndicator::content_offset`. */
    function contentOffset(col) {
        const { x, y } = extraSize(col);
        return { x, y };
    }

    // Tiles and columns.

    /**
     * A window in a column.
     *
     * `height` is niri's `WindowHeight`: `{ kind: "auto", weight }` shares the height left in the
     * column, `{ kind: "fixed", px }` is a window height, `{ kind: "preset", idx }` one of the
     * preset window heights. Only one tile in a column can be non-auto.
     *
     * `request` is the size the engine wants for the tile, and `size` the size it takes, which is
     * what everything is laid out from.
     */
    function makeTile(win, height = { kind: "auto", weight: 1 }) {
        return {
            win,
            height,
            request: null,
            size: { w: 0, h: 0 },
            // The size the window took instead of `request`, for windows that don't take theirs.
            taken: null,
            // What the window last reported, to tell what changed in `windowChanged`.
            seen: { size: { ...win.size }, limits: limitsKey(win) },
            // Size at the end of the last call, see `commit()`.
            committedSize: null,
        };
    }

    function limitsKey(win) {
        return `${sizeKey(win.minSize)} ${sizeKey(win.maxSize)}`;
    }

    /**
     * A column. `width` is niri's `ColumnWidth`, `{ proportion }` of the working area or
     * `{ fixed }` tile pixels; it's kept while the column is full width, fullscreen or maximized,
     * to go back to.
     */
    function makeColumn(width, isFullWidth) {
        const presetWidthIdx = options("preset-column-widths").findIndex((preset) =>
            sameWidth(columnWidthFromPreset(preset), width),
        );
        return {
            // Tells the tab indicators apart.
            id: nextColumnId++,
            tiles: [],
            activeTileIdx: 0,
            width,
            // Matching the width to a preset right away makes the first switch to a preset width
            // go to the next one, even if the window rounded its size down, like terminals do.
            presetWidthIdx: presetWidthIdx === -1 ? null : presetWidthIdx,
            isFullWidth,
            pendingFullscreen: false,
            // Can be set together with fullscreen: leaving fullscreen goes back to maximized.
            pendingMaximized: false,
            display: options("default-column-display"),
            // Sizing mode at the end of the last call, see `commit()`.
            committedMode: null,
        };
    }

    /** niri's `Column::new_with_tile`. */
    function columnWithTile(tile, width, isFullWidth, mode = "normal") {
        const col = makeColumn(width, isFullWidth);
        // Like niri, the window rules can make a new column tabbed.
        col.display = rulesDisplay(tile.win.rules);
        addTileAt(col, 0, tile);
        if (mode === "maximized") col.pendingMaximized = true;
        if (mode === "fullscreen") col.pendingFullscreen = true;
        updateTileSizes(col);
        markCommitted(col);
        return col;
    }

    function markCommitted(col) {
        col.committedMode = sizingMode(col);
        for (const tile of col.tiles) tile.committedSize = { ...tile.size };
    }

    function sizingMode(col) {
        if (col.pendingFullscreen) return "fullscreen";
        if (col.pendingMaximized) return "maximized";
        return "normal";
    }

    /** Width of the widest tile. */
    function tilesWidth(col) {
        return Math.max(...col.tiles.map((tile) => tile.size.w));
    }

    /** Width the column takes on the strip, with the tab indicator. */
    function columnWidth(col) {
        const indicator =
            col.display === "tabbed" && sizingMode(col) === "normal" ? extraSize(col).w : 0;
        return tilesWidth(col) + indicator;
    }

    function find(win) {
        for (let colIdx = 0; colIdx < columns.length; colIdx++) {
            const tileIdx = columns[colIdx].tiles.findIndex((tile) => tile.win.id === win.id);
            if (tileIdx !== -1) return [colIdx, tileIdx];
        }
        return null;
    }

    function activeWindow() {
        if (columns.length === 0) return null;
        const col = columns[activeColumnIdx];
        return col.tiles[col.activeTileIdx].win;
    }

    // Sizes.

    /**
     * Computes the size of every tile in a column, niri's `update_tile_sizes`.
     *
     * The column width applies to every tile, within their min and max widths. Fixed and preset
     * heights are taken first, and auto heights share the rest by weight, growing tiles that would
     * go below their min height.
     */
    function updateTileSizes(col) {
        const mode = sizingMode(col);
        if (mode === "fullscreen" || mode === "maximized") {
            const area = mode === "fullscreen" ? viewSize() : parentArea();
            for (const tile of col.tiles) {
                tile.request = { w: Math.round(area.w), h: Math.round(area.h) };
                settle(tile, mode);
            }
            return;
        }

        const area = workingArea();
        const gap = gaps();
        const isTabbed = col.display === "tabbed";
        const mins = col.tiles.map(minSize);
        const maxs = col.tiles.map((tile) => tile.win.maxSize);

        const minWidth = Math.max(...mins.map((size) => size.w));
        const maxWidth = Math.max(Math.min(...maxs.map((size) => size.w || Infinity)), minWidth);
        const extra = extraSize(col);
        let width = resolveColumnWidth(col, col.isFullWidth ? { proportion: 1 } : col.width);
        width = Math.max(Math.min(width, maxWidth), minWidth);
        const maxTileHeight = area.h - gap * 2 - extra.h;

        // With several windows in a column, the non-auto one can't push the others below their
        // min heights.
        let maxNonAutoWindowHeight = null;
        if (col.tiles.length > 1 && !isTabbed) {
            const nonAutoIdx = col.tiles.findIndex((tile) => tile.height.kind !== "auto");
            if (nonAutoIdx !== -1) {
                const minHeightTaken = mins
                    .filter((_, idx) => idx !== nonAutoIdx)
                    .reduce((sum, min) => sum + min.h + gap, 0);
                const tile = col.tiles[nonAutoIdx];
                const heightLeft = maxTileHeight - minHeightTaken;
                maxNonAutoWindowHeight = Math.max(
                    1,
                    Math.round(windowHeightForTileHeight(tile, heightLeft)),
                );
            }
        }

        // Start by converting window heights to tile heights.
        const heights = col.tiles.map((tile) => {
            const height = tile.height;
            if (height.kind === "auto") return { ...height };

            let windowHeight;
            if (height.kind === "fixed") {
                windowHeight = Math.max(1, Math.round(height.px));
                const max =
                    maxNonAutoWindowHeight ??
                    Math.round(windowHeightForTileHeight(tile, maxTileHeight));
                windowHeight = Math.min(windowHeight, max);
            } else {
                const preset = options("preset-window-heights")[height.idx];
                const resolved = resolvePresetSize(preset, area.h, extra.h);
                windowHeight =
                    resolved.tile !== undefined
                        ? windowHeightForTileHeight(tile, resolved.tile)
                        : resolved.window;
                windowHeight = clamp(Math.round(windowHeight), 1, MAX_PX);
                if (maxNonAutoWindowHeight !== null) {
                    windowHeight = Math.min(windowHeight, maxNonAutoWindowHeight);
                }
            }
            return { kind: "fixed", px: tileHeightForWindowHeight(tile, windowHeight) };
        });

        // In tabbed display, every tab has the height of the fixed one, or the full height.
        if (isTabbed) {
            let tabbedHeight = heights.find((h) => h.kind === "fixed")?.px ?? maxTileHeight;
            // Take min heights into account, but don't force every tab as tall as a tab that's
            // taller than the working area.
            const minHeight = Math.min(maxTileHeight, Math.max(...mins.map((min) => min.h)));
            tabbedHeight = Math.max(tabbedHeight, minHeight);
            heights.forEach((_, idx) => (heights[idx] = { kind: "fixed", px: tabbedHeight }));
        }

        let heightLeft = area.h - gap * (col.tiles.length + 1);
        let autoTilesLeft = col.tiles.length;

        // Subtract all fixed heights.
        heights.forEach((h, idx) => {
            const min = mins[idx];
            const max = maxs[idx];
            // A window with an exact height constraint gets it.
            if (min.h === max.h) heights[idx] = h = { kind: "fixed", px: min.h };
            if (h.kind === "fixed") {
                if (max.h > 0) h.px = Math.min(h.px, max.h);
                h.px = Math.max(h.px, min.h);
                heightLeft -= h.px;
                autoTilesLeft -= 1;
            }
        });

        let totalWeight = heights
            .filter((h) => h.kind === "auto")
            .reduce((sum, h) => sum + h.weight, 0);

        // Window heights are whole pixels, so auto heights are rounded one by one as they're
        // handed out. Tiles that would go below their min height get it as a fixed height, which
        // leaves less for the rest, so then start over. Max heights aren't respected here: most of
        // them are dialogs with min = max, handled above.
        const roundHeight = (tile, h) =>
            tileHeightForWindowHeight(
                tile,
                Math.max(1, Math.round(windowHeightForTileHeight(tile, h))),
            );
        outer: while (autoTilesLeft > 0) {
            let heightLeft2 = heightLeft;
            let totalWeight2 = totalWeight;
            for (let idx = 0; idx < heights.length; idx++) {
                const h = heights[idx];
                if (h.kind !== "auto") continue;
                const auto = (heightLeft2 * h.weight) / totalWeight2;
                if (mins[idx].h > auto) {
                    heights[idx] = { kind: "fixed", px: mins[idx].h };
                    heightLeft -= mins[idx].h;
                    totalWeight -= h.weight;
                    autoTilesLeft -= 1;
                    continue outer;
                }
                heightLeft2 -= roundHeight(col.tiles[idx], auto);
                totalWeight2 -= h.weight;
            }

            // All min heights are satisfied, fill them in.
            for (let idx = 0; idx < heights.length; idx++) {
                const h = heights[idx];
                if (h.kind !== "auto") continue;
                const auto = roundHeight(col.tiles[idx], (heightLeft * h.weight) / totalWeight);
                heights[idx] = { kind: "fixed", px: auto };
                heightLeft -= auto;
                totalWeight -= h.weight;
                autoTilesLeft -= 1;
            }
        }

        col.tiles.forEach((tile, idx) => {
            // Windows get whole-pixel sizes, rounded down so proportional columns don't overflow.
            const { borders } = tile.win;
            tile.request = {
                w: Math.floor(Math.max(1, width - borders.w)) + borders.w,
                h: Math.floor(Math.max(1, heights[idx].px - borders.h)) + borders.h,
            };
            settle(tile, mode);
        });
    }

    /** Works out the size a tile will have: what it asks for, unless the window took another. */
    function settle(tile, mode) {
        const { request, win } = tile;
        let size;
        if (tile.taken !== null && tile.taken.for === sizeKey(request)) {
            size = { w: tile.taken.w, h: tile.taken.h };
        } else if (mode === "fullscreen") {
            size = { ...request };
        } else {
            // The window would clamp the size to its limits anyway.
            size = {
                w: applyLimits(request.w, win.minSize.w, win.maxSize.w),
                h: applyLimits(request.h, win.minSize.h, win.maxSize.h),
            };
        }
        if (mode === "fullscreen") {
            // A fullscreen tile covers the view even if the window is smaller.
            size.w = Math.max(size.w, viewSize().w);
            size.h = Math.max(size.h, viewSize().h);
        }
        tile.size = size;
    }

    function updateAllTileSizes() {
        for (const col of columns) updateTileSizes(col);
    }

    // Positions.

    /** Canvas x of a column, or one past the last column. */
    function columnX(idx) {
        let x = 0;
        for (let i = 0; i < idx; i++) x += columnWidth(columns[i]) + gaps();
        return x;
    }

    function tilesOrigin(col) {
        switch (sizingMode(col)) {
            case "fullscreen":
                return { x: 0, y: 0 };
            case "maximized":
                return { x: 0, y: parentArea().y };
            default: {
                const offset = contentOffset(col);
                return { x: offset.x, y: workingArea().y + gaps() + offset.y };
            }
        }
    }

    /** Offsets of the tiles from the column's x, and one past the last tile. */
    function tileOffsets(col) {
        const center = options("center-focused-column") === "always";
        const isTabbed = col.display === "tabbed";
        const width = tilesWidth(col);
        const origin = tilesOrigin(col);
        const offsets = [];
        for (const size of [...col.tiles.map((tile) => tile.size), { w: 0, h: 0 }]) {
            const pos = { ...origin };
            if (center) pos.x += (width - size.w) / 2;
            if (!isTabbed) origin.y += size.h + gaps();
            offsets.push(pos);
        }
        return offsets;
    }

    // The view.

    function isCenteringFocusedColumn() {
        return (
            options("center-focused-column") === "always" ||
            (options("always-center-single-column") && columns.length <= 1)
        );
    }

    function targetViewPos() {
        return columnX(activeColumnIdx) + viewOffset;
    }

    /** View offset that shows a column with as little movement as possible. */
    function computeNewViewOffsetFit(targetX, colX, width, mode) {
        if (mode === "fullscreen") return 0;
        const area = mode === "maximized" ? parentArea() : workingArea();
        const padding = mode === "maximized" ? 0 : gaps();
        targetX ??= targetViewPos();
        // Non-fullscreen windows are always offset at least by the working area position.
        return computeNewViewOffset(targetX + area.x, area.w, colX, width, padding) - area.x;
    }

    function computeNewViewOffsetCentered(targetX, colX, width, mode) {
        if (mode === "fullscreen") return computeNewViewOffsetFit(targetX, colX, width, mode);
        const area = mode === "maximized" ? parentArea() : workingArea();
        // Columns wider than the view are left-aligned (the fit code can deal with that).
        if (area.w <= width) return computeNewViewOffsetFit(targetX, colX, width, mode);
        return -(area.w - width) / 2 - area.x;
    }

    function computeNewViewOffsetForColumnFit(targetX, idx) {
        const col = columns[idx];
        return computeNewViewOffsetFit(targetX, columnX(idx), columnWidth(col), sizingMode(col));
    }

    function computeNewViewOffsetForColumnCentered(targetX, idx) {
        const col = columns[idx];
        return computeNewViewOffsetCentered(
            targetX,
            columnX(idx),
            columnWidth(col),
            sizingMode(col),
        );
    }

    /** View offset for a column that's being activated, coming from `prevIdx`. */
    function computeNewViewOffsetForColumn(targetX, idx, prevIdx) {
        if (isCenteringFocusedColumn()) {
            return computeNewViewOffsetForColumnCentered(targetX, idx);
        }
        if (
            options("center-focused-column") !== "on-overflow" ||
            prevIdx === null ||
            prevIdx === idx
        ) {
            return computeNewViewOffsetForColumnFit(targetX, idx);
        }

        // Always take the left or right neighbor of the target as the source.
        const sourceIdx =
            prevIdx > idx ? Math.min(idx + 1, columns.length - 1) : Math.max(idx - 1, 0);
        const sourceColX = columnX(sourceIdx);
        const sourceColWidth = columnWidth(columns[sourceIdx]);
        const targetColX = columnX(idx);
        const targetColWidth = columnWidth(columns[idx]);
        const totalWidth =
            (sourceColX < targetColX
                ? targetColX - sourceColX + targetColWidth
                : sourceColX - targetColX + sourceColWidth) +
            gaps() * 2;

        // If it fits together, do a normal scroll, otherwise center the new column.
        return totalWidth <= workingArea().w
            ? computeNewViewOffsetForColumnFit(targetX, idx)
            : computeNewViewOffsetForColumnCentered(targetX, idx);
    }

    /** Scrolls the view to show the active column, niri's `animate_view_offset_to_column`. */
    function fitView() {
        viewOffset = computeNewViewOffsetForColumn(null, activeColumnIdx, null);
    }

    function activateColumn(idx) {
        if (activeColumnIdx === idx) return;
        viewOffset = computeNewViewOffsetForColumn(null, idx, activeColumnIdx);
        activeColumnIdx = idx;
        // A different column was activated; reset the flags.
        activatePrevColumnOnRemoval = null;
        viewOffsetToRestore = null;
        interactiveResize = null;
    }

    function cancelResizeForColumn(col) {
        if (
            interactiveResize !== null &&
            col.tiles.some((tile) => tile.win === interactiveResize.win)
        ) {
            interactiveResize = null;
        }
    }

    // Adding and removing.

    function addColumn(idx, col, activate) {
        const wasEmpty = columns.length === 0;
        idx ??= wasEmpty ? 0 : activeColumnIdx + 1;
        columns.splice(idx, 0, col);
        if (!wasEmpty && idx <= activeColumnIdx) activeColumnIdx += 1;

        if (activate) {
            // The first window on an empty workspace drops whatever view offset was left over,
            // without scrolling there.
            if (wasEmpty) {
                activeColumnIdx = idx;
                viewOffset = 0;
                viewOffset = computeNewViewOffsetForColumn(null, idx, null);
                skipCameraAnimation = true;
            }
            const prevOffset = !wasEmpty && idx === activeColumnIdx + 1 ? viewOffset : null;
            activateColumn(idx);
            activatePrevColumnOnRemoval = prevOffset;
        }
    }

    function addTile(colIdx, tile, activate, width, isFullWidth, mode) {
        addColumn(colIdx, columnWithTile(tile, width, isFullWidth, mode), activate);
    }

    function addTileAt(col, idx, tile) {
        // Only tabbed columns can be fullscreen with several windows.
        if (col.display !== "tabbed") {
            col.pendingFullscreen = false;
            col.pendingMaximized = false;
        }
        tile.height = { kind: "auto", weight: 1 };
        col.tiles.splice(idx, 0, tile);
        updateTileSizes(col);
    }

    function addTileToColumn(colIdx, tileIdx, tile, activate) {
        const col = columns[colIdx];
        tileIdx ??= col.tiles.length;
        const prevActiveTileIdx = col.activeTileIdx;
        addTileAt(col, tileIdx, tile);
        if (tileIdx <= prevActiveTileIdx) col.activeTileIdx += 1;
        if (activate) {
            col.activeTileIdx = tileIdx;
            activateColumn(colIdx);
        }
    }

    /** Removes a tile, and its column if it was the last one. Returns what the tile had. */
    function removeTile(colIdx, tileIdx) {
        const col = columns[colIdx];
        const removed = {
            width: col.width,
            isFullWidth: col.isFullWidth,
            mode: sizingMode(col),
        };
        if (col.tiles.length === 1) {
            removeColumn(colIdx);
            return { tile: col.tiles[0], ...removed };
        }

        const [tile] = col.tiles.splice(tileIdx, 1);
        // If one window is left, reset its weight to 1.
        if (col.tiles.length === 1 && col.tiles[0].height.kind === "auto") {
            col.tiles[0].height = { kind: "auto", weight: 1 };
        }
        if (interactiveResize?.win === tile.win) interactiveResize = null;

        if (tileIdx < col.activeTileIdx) {
            // A tile above was removed; keep the same one active.
            col.activeTileIdx -= 1;
        } else if (tileIdx === col.activeTileIdx && tileIdx === col.tiles.length) {
            // The bottom tile was removed and it was active.
            col.activeTileIdx -= 1;
        }
        updateTileSizes(col);
        return { tile, ...removed };
    }

    function removeColumn(colIdx) {
        const [col] = columns.splice(colIdx, 1);
        cancelResizeForColumn(col);

        if (colIdx + 1 === activeColumnIdx) {
            // The previous column, that we were going to activate upon removal of the active
            // column, has just been itself removed.
            activatePrevColumnOnRemoval = null;
        }
        if (colIdx === activeColumnIdx) viewOffsetToRestore = null;
        if (columns.length === 0) return col;

        if (colIdx < activeColumnIdx) {
            // A column to the left was removed; keep the same one active.
            activeColumnIdx -= 1;
            activatePrevColumnOnRemoval = null;
        } else if (colIdx === activeColumnIdx && activatePrevColumnOnRemoval !== null) {
            // The active column was removed, and we need to activate the previous one.
            if (colIdx > 0) {
                const prevOffset = activatePrevColumnOnRemoval;
                activateColumn(activeColumnIdx - 1);
                // Restore the view offset, but scroll in case the previous window has resized.
                viewOffset = prevOffset;
                fitView();
            }
        } else {
            // The next column takes the place of the removed one, or the previous if there's none.
            activateColumn(Math.min(activeColumnIdx, columns.length - 1));
        }
        return col;
    }

    /** Width for a column that a window arrives in, from the window rules. */
    function defaultWidth(win) {
        const preset = rulesWidth(win.rules);
        if (preset === null) {
            // Let the window pick, so use the size it has, unless it doesn't have one yet.
            if (win.size.w > 0) return { fixed: win.size.w };
            return columnWidthFromPreset(options("default-column-width") ?? { proportion: 0.5 });
        }
        if ("proportion" in preset) return { proportion: preset.proportion };
        // niri's column widths include borders, fixed presets don't.
        return { fixed: preset.fixed + win.borders.w };
    }

    /**
     * Width for a column of windows that arrive with a size of their own: a preset width if it
     * matches one, so the column keeps resizing with the view and switching presets continues
     * from it, or else the width they have.
     */
    function widthForSize(w, win) {
        if (w <= 0) return defaultWidth(win);
        const area = workingArea();
        const presets = [...options("preset-column-widths"), { proportion: 1 }];
        const extra = newColumnExtraSize().w;
        for (const preset of presets) {
            const resolved = resolvePresetSize(preset, area.w, extra);
            const px = resolved.tile ?? resolved.window + win.borders.w;
            if (Math.abs(Math.floor(px) - w) <= 1) return columnWidthFromPreset(preset);
        }
        return { fixed: w };
    }

    function addWindow(win, how) {
        const tile = makeTile(win);
        switch (how.reason) {
            case "drop": {
                // niri takes windows out of fullscreen and maximized when a drag starts, so they
                // land as normal windows.
                const position = how.pointer ? insertPosition(how.pointer) : { newColumn: null };
                const kept = dragged?.id === win.id ? dragged : null;
                dragged = null;
                if ("inColumn" in position) {
                    addTileToColumn(position.inColumn, position.tile, tile, how.activate);
                } else {
                    const width = kept?.width ?? widthForSize(win.size.w, win);
                    addTile(
                        position.newColumn,
                        tile,
                        how.activate,
                        width,
                        kept?.isFullWidth ?? false,
                        "normal",
                    );
                }
                return;
            }
            case "open": {
                const nextTo = how.nextTo === null ? null : find(how.nextTo);
                const colIdx = nextTo === null ? null : nextTo[0] + 1;
                const fullWidth = win.rules["open-maximized"] === true;
                addTile(colIdx, tile, how.activate, defaultWidth(win), fullWidth, win.mode);
                if (rulesHeight(win.rules) !== null) {
                    const [newColIdx, tileIdx] = find(win);
                    const change = sizeChangeFromPreset(rulesHeight(win.rules));
                    setWindowHeight(columns[newColIdx], change, tileIdx);
                }
                return;
            }
            case "tile": {
                // Like niri: a window that stops floating keeps its floating width.
                const w = win.size.w || how.rect?.w || 0;
                const width = w > 0 ? { fixed: w } : defaultWidth(win);
                addTile(null, tile, how.activate, width, false, win.mode);
                return;
            }
            default: {
                // "move" and "adopt": the window comes with a size from somewhere else.
                const w = win.size.w || how.rect?.w || 0;
                addTile(null, tile, how.activate, widthForSize(w, win), false, win.mode);
            }
        }
    }

    function removeWindow(win, reason) {
        const found = find(win);
        if (found === null) return;
        const [colIdx, tileIdx] = found;
        const col = columns[colIdx];
        if (reason === "drag") {
            dragged = { id: win.id, width: col.width, isFullWidth: col.isFullWidth };
        }
        removeTile(colIdx, tileIdx);
    }

    // Focus.

    function focusLeft() {
        if (activeColumnIdx === 0) return false;
        activateColumn(activeColumnIdx - 1);
        return true;
    }

    function focusRight() {
        if (activeColumnIdx + 1 >= columns.length) return false;
        activateColumn(activeColumnIdx + 1);
        return true;
    }

    function focusColumnFirst() {
        if (columns.length > 0) activateColumn(0);
    }

    function focusColumnLast() {
        if (columns.length > 0) activateColumn(columns.length - 1);
    }

    /** Activates a tile in a column; returns whether it changed. */
    function activateTile(col, idx) {
        if (col.activeTileIdx === idx) return false;
        col.activeTileIdx = idx;
        return true;
    }

    function focusUp() {
        if (columns.length === 0) return false;
        const col = columns[activeColumnIdx];
        return activateTile(col, Math.max(col.activeTileIdx - 1, 0));
    }

    function focusDown() {
        if (columns.length === 0) return false;
        const col = columns[activeColumnIdx];
        return activateTile(col, Math.min(col.activeTileIdx + 1, col.tiles.length - 1));
    }

    function focusTop() {
        if (columns.length > 0) activateTile(columns[activeColumnIdx], 0);
    }

    function focusBottom() {
        if (columns.length === 0) return;
        const col = columns[activeColumnIdx];
        activateTile(col, col.tiles.length - 1);
    }

    function activateWindow(win) {
        const found = find(win);
        if (found === null) return;
        activateTile(columns[found[0]], found[1]);
        activateColumn(found[0]);
    }

    // Moving columns and windows.

    function moveColumnTo(newIdx) {
        if (activeColumnIdx === newIdx) return;
        const currentColX = columnX(activeColumnIdx);
        const [col] = columns.splice(activeColumnIdx, 1);
        cancelResizeForColumn(col);
        columns.splice(newIdx, 0, col);
        // Keep the camera where it is while the column moves into view.
        viewOffset += currentColX - columnX(activeColumnIdx);
        activateColumn(newIdx);
    }

    function moveLeft() {
        if (activeColumnIdx === 0) return false;
        moveColumnTo(activeColumnIdx - 1);
        return true;
    }

    function moveRight() {
        if (activeColumnIdx + 1 >= columns.length) return false;
        moveColumnTo(activeColumnIdx + 1);
        return true;
    }

    /** Moves the active window up or down in its column. */
    function moveInColumn(delta) {
        if (columns.length === 0) return false;
        const col = columns[activeColumnIdx];
        const idx = col.activeTileIdx;
        const newIdx = clamp(idx + delta, 0, col.tiles.length - 1);
        if (newIdx === idx) return false;
        [col.tiles[idx], col.tiles[newIdx]] = [col.tiles[newIdx], col.tiles[idx]];
        col.activeTileIdx = newIdx;
        updateTileSizes(col);
        return true;
    }

    function sourceTile(win) {
        if (win !== null) return find(win);
        return [activeColumnIdx, columns[activeColumnIdx].activeTileIdx];
    }

    function consumeOrExpelWindowLeft(win) {
        if (columns.length === 0) return;
        const [sourceColIdx, sourceTileIdx] = sourceTile(win);
        const source = columns[sourceColIdx];
        const sourceTileWasActive =
            activeColumnIdx === sourceColIdx && source.activeTileIdx === sourceTileIdx;

        if (source.tiles.length === 1) {
            if (sourceColIdx === 0) return;
            // Move into the column on the left.
            const targetColIdx = sourceColIdx - 1;
            if (sourceTileWasActive) {
                // Activate the target column when the source goes away, without moving the view.
                // If it was already going to be activated, leave the offset as is: this keeps the
                // common "open a window, consume it left as a new tab" from scrolling.
                activatePrevColumnOnRemoval ??=
                    viewOffset + columnX(sourceColIdx) - columnX(targetColIdx);
            }
            const { tile } = removeTile(sourceColIdx, 0);
            addTileToColumn(targetColIdx, null, tile, sourceTileWasActive);
        } else {
            // Move out of the column, into a new one on the left.
            const removed = removeTile(sourceColIdx, sourceTileIdx);
            addTile(
                sourceColIdx,
                removed.tile,
                sourceTileWasActive,
                removed.width,
                removed.isFullWidth,
                removed.mode,
            );
            // We added to the left, don't activate even further left on removal.
            if (sourceTileWasActive) activatePrevColumnOnRemoval = null;
        }
    }

    function consumeOrExpelWindowRight(win) {
        if (columns.length === 0) return;
        const [sourceColIdx, sourceTileIdx] = sourceTile(win);
        const source = columns[sourceColIdx];
        const sourceTileWasActive =
            activeColumnIdx === sourceColIdx && source.activeTileIdx === sourceTileIdx;

        if (source.tiles.length === 1) {
            if (sourceColIdx + 1 === columns.length) return;
            // Move into the column on the right, which takes the source's place.
            if (sourceTileWasActive) activatePrevColumnOnRemoval = null;
            const { tile } = removeTile(sourceColIdx, 0);
            addTileToColumn(sourceColIdx, null, tile, sourceTileWasActive);
        } else {
            // Move out of the column, into a new one on the right.
            const removed = removeTile(sourceColIdx, sourceTileIdx);
            addTile(
                sourceColIdx + 1,
                removed.tile,
                sourceTileWasActive,
                removed.width,
                removed.isFullWidth,
                removed.mode,
            );
        }
    }

    function consumeIntoColumn() {
        if (columns.length < 2 || activeColumnIdx === columns.length - 1) return;
        const { tile } = removeTile(activeColumnIdx + 1, 0);
        addTileToColumn(activeColumnIdx, null, tile, false);
    }

    function expelFromColumn() {
        if (columns.length === 0) return;
        const source = columns[activeColumnIdx];
        if (source.tiles.length === 1) return;
        const removed = removeTile(activeColumnIdx, source.tiles.length - 1);
        addTile(
            activeColumnIdx + 1,
            removed.tile,
            false,
            removed.width,
            removed.isFullWidth,
            removed.mode,
        );
    }

    function swapWindowInDirection(direction) {
        if (columns.length === 0) return;
        const step = direction === "left" ? -1 : 1;
        const sourceColIdx = activeColumnIdx;
        const targetColIdx = activeColumnIdx + step;
        if (targetColIdx < 0 || targetColIdx >= columns.length) return;

        // With a single tile on both sides, this is a column move.
        if (columns[sourceColIdx].tiles.length === 1 && columns[targetColIdx].tiles.length === 1) {
            moveColumnTo(targetColIdx);
            return;
        }

        const sourceTileIdx = columns[sourceColIdx].activeTileIdx;
        const targetTileIdx = columns[targetColIdx].activeTileIdx;
        const sourceColumnDrained = columns[sourceColIdx].tiles.length === 1;

        // Removing the only tile of the source column removes the column; it's created again
        // below with the target tile.
        const sourceRemoved = removeTile(sourceColIdx, sourceTileIdx);
        const adjustedTargetColIdx =
            direction === "right" && sourceColumnDrained ? targetColIdx - 1 : targetColIdx;
        addTileToColumn(adjustedTargetColIdx, targetTileIdx, sourceRemoved.tile, false);
        const { tile: targetTile } = removeTile(adjustedTargetColIdx, targetTileIdx + 1);
        if (sourceColumnDrained) {
            addTile(
                sourceColIdx,
                targetTile,
                true,
                sourceRemoved.width,
                sourceRemoved.isFullWidth,
                sourceRemoved.mode,
            );
        } else {
            addTileToColumn(sourceColIdx, sourceTileIdx, targetTile, false);
        }

        columns[sourceColIdx].activeTileIdx = sourceTileIdx;
        columns[targetColIdx].activeTileIdx = targetTileIdx;
        activateColumn(targetColIdx);
    }

    // Display, centering and sizes.

    function setColumnDisplay(display) {
        if (columns.length === 0) return;
        const col = columns[activeColumnIdx];
        if (col.display === display) return;
        cancelResizeForColumn(col);
        col.display = display;
        updateTileSizes(col);
        // Only tabbed columns can be fullscreen with several windows.
        if (display !== "tabbed" && col.tiles.length > 1) {
            const win = col.tiles[col.activeTileIdx].win;
            setFullscreen(win, false);
            setMaximized(win, false);
        }
    }

    function centerColumn() {
        if (columns.length === 0) return;
        viewOffset = computeNewViewOffsetForColumnCentered(null, activeColumnIdx);
        cancelResizeForColumn(columns[activeColumnIdx]);
    }

    function centerWindow(win) {
        if (columns.length === 0) return;
        // We can reasonably center only the active column.
        const colIdx = win === null ? activeColumnIdx : find(win)[0];
        if (colIdx === activeColumnIdx) centerColumn();
    }

    /**
     * Columns that fit in the working area at the current view position, for
     * `center-visible-columns` and `expand-column-to-available-width`.
     */
    function fullyVisibleColumns() {
        const viewX = targetViewPos();
        const area = workingArea();
        const gap = gaps();
        let widthTaken = 0;
        let leftmostColX = null;
        let activeColX = null;
        let countedNonActive = false;
        for (let idx = 0; idx < columns.length; idx++) {
            const colX = columnX(idx);
            // Column goes off-screen to the left.
            if (colX < viewX + area.x + gap) continue;
            leftmostColX ??= colX;
            const width = columnWidth(columns[idx]);
            // Column goes off-screen to the right. We can stop here.
            if (viewX + area.x + area.w < colX + width + gap) break;
            if (idx === activeColumnIdx) activeColX = colX;
            else countedNonActive = true;
            widthTaken += width + gap;
        }
        return { widthTaken, leftmostColX, activeColX, countedNonActive };
    }

    function centerVisibleColumns() {
        if (columns.length === 0 || isCenteringFocusedColumn()) return;
        const { widthTaken, leftmostColX, activeColX } = fullyVisibleColumns();
        // The active column isn't fully on screen, so we can't meaningfully do anything.
        if (activeColX === null) return;
        cancelResizeForColumn(columns[activeColumnIdx]);
        const area = workingArea();
        const freeSpace = area.w - widthTaken + gaps();
        const newViewX = leftmostColX - freeSpace / 2 - area.x;
        viewOffset = newViewX - activeColX;
        fitView();
    }

    function toggleFullWidth(col) {
        if (col.pendingMaximized) {
            // Treat it as unmaximize.
            col.pendingMaximized = false;
            col.isFullWidth = false;
        } else {
            col.isFullWidth = !col.isFullWidth;
        }
        updateTileSizes(col);
    }

    function setColumnWidth(col, change, tileIdx) {
        const current = col.isFullWidth || col.pendingMaximized ? { proportion: 1 } : col.width;
        const currentPx = resolveColumnWidth(col, current);
        let width;
        if (change.op === "set" && change.unit === "px") {
            // Setting a fixed width gives that width to the window, usually the active one.
            const tile = col.tiles[tileIdx ?? col.activeTileIdx];
            width = { fixed: clamp(tileWidthForWindowWidth(tile, change.value), 1, MAX_PX) };
        } else if (change.op === "set") {
            width = { proportion: clamp(change.value / 100, 0, MAX_F) };
        } else if (change.unit === "px") {
            width = { fixed: clamp(currentPx + change.value, 1, MAX_PX) };
        } else if ("proportion" in current) {
            width = { proportion: clamp(current.proportion + change.value / 100, 0, MAX_F) };
        } else {
            const full = workingArea().w - gaps();
            const proportion = full === 0 ? 1 : (currentPx + gaps() + extraSize(col).w) / full;
            width = { proportion: clamp(proportion + change.value / 100, 0, MAX_F) };
        }

        col.width = width;
        col.presetWidthIdx = null;
        col.isFullWidth = false;
        col.pendingMaximized = false;
        updateTileSizes(col);
    }

    /** `switch-preset-column-width` and `switch-preset-window-width`. */
    function toggleWidth(col, tileIdx, forwards) {
        const presets = options("preset-column-widths");
        if (presets.length === 0) return;
        const tile = col.tiles[tileIdx ?? col.activeTileIdx];
        const len = presets.length;
        let presetIdx = col.isFullWidth || col.pendingMaximized ? null : col.presetWidthIdx;
        if (presetIdx !== null) {
            presetIdx = (presetIdx + (forwards ? 1 : len - 1)) % len;
        } else {
            // Go to the next preset from the current width, with some allowance for fractional
            // scaling.
            const currentTile = tile.size.w;
            const currentWindow = windowWidthForTileWidth(tile, currentTile);
            const area = workingArea();
            const fits = presets.map((preset) => {
                const resolved = resolvePresetSize(preset, area.w, extraSize(col).w);
                return resolved.tile !== undefined
                    ? { value: resolved.tile, current: currentTile }
                    : { value: resolved.window, current: currentWindow };
            });
            if (forwards) {
                presetIdx = fits.findIndex(({ value, current }) => current + 1 < value);
                if (presetIdx === -1) presetIdx = 0;
            } else {
                presetIdx = fits.findLastIndex(({ value, current }) => value + 1 < current);
                if (presetIdx === -1) presetIdx = len - 1;
            }
        }
        setColumnWidth(col, sizeChangeFromPreset(presets[presetIdx]), tileIdx);
        col.presetWidthIdx = presetIdx;
    }

    function setWindowHeight(col, change, tileIdx) {
        tileIdx ??= col.activeTileIdx;
        // Only one window in a column can have a non-auto height, so start by converting every
        // height to auto. A tile that's already non-auto can skip that, which keeps the weights
        // when a resize makes other windows hit their min height and then goes back.
        if (col.tiles[tileIdx].height.kind === "auto") convertHeightsToAuto(col);

        const tile = col.tiles[tileIdx];
        const area = workingArea();
        const gap = gaps();
        const extra = extraSize(col).h;
        const currentWindowPx =
            tile.height.kind === "fixed"
                ? tile.height.px
                : windowHeightForTileHeight(tile, tile.size.h);
        const currentTilePx = tileHeightForWindowHeight(tile, currentWindowPx);
        const full = area.h - gap;
        const currentProp = full === 0 ? 1 : (currentTilePx + gap) / full;

        let windowHeight;
        if (change.op === "set" && change.unit === "px") {
            windowHeight = change.value;
        } else if (change.op === "set") {
            windowHeight = windowHeightForTileHeight(
                tile,
                (area.h - gap) * (change.value / 100) - gap - extra,
            );
        } else if (change.unit === "px") {
            windowHeight = currentWindowPx + change.value;
        } else {
            const proportion = currentProp + change.value / 100;
            windowHeight = windowHeightForTileHeight(
                tile,
                (area.h - gap) * proportion - gap - extra,
            );
        }

        // Leave room for the other windows' min heights, or just the working area.
        const minHeightTaken =
            col.display === "tabbed"
                ? 0
                : col.tiles
                      .filter((_, idx) => idx !== tileIdx)
                      .reduce((sum, other) => sum + minSize(other).h + gap, 0);
        const heightLeft = Math.max(
            1,
            windowHeightForTileHeight(tile, area.h - extra - gap - minHeightTaken - gap),
        );
        windowHeight = Math.min(heightLeft, windowHeight);

        // Clamp it against the window's own limits.
        const { minSize: min, maxSize: max } = tile.win;
        if (max.h > 0)
            windowHeight = Math.min(windowHeight, windowHeightForTileHeight(tile, max.h));
        if (min.h > 0)
            windowHeight = Math.max(windowHeight, windowHeightForTileHeight(tile, min.h));

        tile.height = { kind: "fixed", px: clamp(windowHeight, 1, MAX_PX) };
        col.pendingMaximized = false;
        updateTileSizes(col);
    }

    function resetWindowHeight(col, tileIdx) {
        if (col.display === "tabbed") {
            // When tabbed, reset any window, not just the one with the fixed height.
            for (const tile of col.tiles) tile.height = { kind: "auto", weight: 1 };
        } else {
            col.tiles[tileIdx ?? col.activeTileIdx].height = { kind: "auto", weight: 1 };
        }
        updateTileSizes(col);
    }

    /** `switch-preset-window-height`. */
    function toggleWindowHeight(col, tileIdx, forwards) {
        const presets = options("preset-window-heights");
        if (presets.length === 0) return;
        tileIdx ??= col.activeTileIdx;
        // Like in `setWindowHeight`, only one window can have a non-auto height.
        if (col.tiles[tileIdx].height.kind === "auto") convertHeightsToAuto(col);

        const tile = col.tiles[tileIdx];
        const len = presets.length;
        let presetIdx;
        if (tile.height.kind === "preset" && !col.pendingMaximized) {
            presetIdx = (tile.height.idx + (forwards ? 1 : len - 1)) % len;
        } else {
            const current = tile.size.h;
            const area = workingArea();
            const resolved = presets.map((preset) => {
                const size = resolvePresetSize(preset, area.h, extraSize(col).h);
                const windowHeight =
                    size.tile !== undefined
                        ? windowHeightForTileHeight(tile, size.tile)
                        : size.window;
                return tileHeightForWindowHeight(tile, clamp(Math.round(windowHeight), 1, MAX_PX));
            });
            // Some allowance for fractional scaling purposes.
            if (forwards) {
                presetIdx = resolved.findIndex((height) => current + 1 < height);
                if (presetIdx === -1) presetIdx = 0;
            } else {
                presetIdx = resolved.findLastIndex((height) => height + 1 < current);
                if (presetIdx === -1) presetIdx = len - 1;
            }
        }
        tile.height = { kind: "preset", idx: presetIdx };
        col.pendingMaximized = false;
        updateTileSizes(col);
    }

    /**
     * Converts every height in a column to auto, keeping the heights the tiles have now.
     *
     * Weights are invariant to multiplication, so they're centered on the median tile height,
     * which gets weight 1.
     */
    function convertHeightsToAuto(col) {
        const heights = col.tiles.map((tile) => tile.size.h);
        const sorted = [...heights].sort((a, b) => a - b);
        const median = sorted[Math.floor(sorted.length / 2)];
        col.tiles.forEach((tile, idx) => {
            tile.height = { kind: "auto", weight: heights[idx] / median };
        });
    }

    function expandColumnToAvailableWidth() {
        if (columns.length === 0) return;
        const col = columns[activeColumnIdx];
        if (sizingMode(col) !== "normal" || col.isFullWidth) return;

        if (isCenteringFocusedColumn()) {
            // The active column stays centered, so there's no width that keeps the neighbors on
            // screen in any useful way. Do the simple thing.
            toggleFullWidth(col);
            cancelResizeForColumn(col);
            return;
        }

        const { widthTaken, leftmostColX, activeColX, countedNonActive } = fullyVisibleColumns();
        // The active column isn't fully on screen, so we can't meaningfully do anything.
        if (activeColX === null) return;
        const area = workingArea();
        const gap = gaps();
        const availableWidth = area.w - gap - widthTaken - extraSize(col).w;
        // Nowhere to expand.
        if (availableWidth <= 0) return;
        cancelResizeForColumn(col);

        if (!countedNonActive) {
            // Only the active column is on screen, so it goes to the full working area width. As
            // full width, it's easier to back out of.
            toggleFullWidth(col);
            return;
        }

        col.width = { fixed: columnWidth(col) + availableWidth };
        col.presetWidthIdx = null;
        col.isFullWidth = false;
        updateTileSizes(col);

        // Put the leftmost window into the view.
        viewOffset = leftmostColX - gap - area.x - activeColX;
        fitView();
    }

    // Fullscreen and maximized.

    function setFullscreen(win, isFullscreen) {
        let [colIdx] = find(win);
        if (columns[colIdx].pendingFullscreen === isFullscreen) return false;
        cancelResizeForColumn(columns[colIdx]);
        const col = columns[colIdx];
        if (isFullscreen && col.tiles.length > 1 && col.display !== "tabbed") {
            // This wasn't the only window in its column; extract it into a separate column.
            consumeOrExpelWindowRight(win);
            colIdx += 1;
        }
        columns[colIdx].pendingFullscreen = isFullscreen;
        updateTileSizes(columns[colIdx]);
        return true;
    }

    function setMaximized(win, maximize) {
        let [colIdx] = find(win);
        if (columns[colIdx].pendingMaximized === maximize) return false;
        cancelResizeForColumn(columns[colIdx]);
        const col = columns[colIdx];
        if (maximize && col.tiles.length > 1 && col.display !== "tabbed") {
            consumeOrExpelWindowRight(win);
            colIdx += 1;
        }
        columns[colIdx].pendingMaximized = maximize;
        updateTileSizes(columns[colIdx]);
        return true;
    }

    // Interactive resize.

    function interactiveResizeUpdate(win, delta) {
        if (interactiveResize === null || interactiveResize.win !== win) return;
        const [colIdx, tileIdx] = find(win);
        const col = columns[colIdx];
        const { edges, originalWindowSize } = interactiveResize;
        const prevWidth = columnWidth(col);

        if (edges.left || edges.right) {
            let dx = edges.left ? -delta.x : delta.x;
            if (isCenteringFocusedColumn()) dx *= 2;
            const value = Math.round(originalWindowSize.w + dx);
            setColumnWidth(col, { op: "set", value, unit: "px" }, tileIdx);
        }
        // Resizing the top edge of the topmost window would be weird, so it doesn't.
        if ((edges.top || edges.bottom) && !(edges.top && tileIdx === 0)) {
            const dy = edges.top ? -delta.y : delta.y;
            const value = Math.round(originalWindowSize.h + dy);
            setWindowHeight(col, { op: "set", value, unit: "px" }, tileIdx);
        }

        // Keep the view still while resizing, or keep the right edge in place when resizing by
        // the left edge.
        const width = columnWidth(col);
        if (colIdx === activeColumnIdx && width !== prevWidth) {
            if (isCenteringFocusedColumn()) {
                const area = workingArea();
                viewOffset = -(area.w - width) / 2 - area.x;
            } else if (edges.left) {
                viewOffset += width - prevWidth;
            }
        }
    }

    // Swipes.

    /**
     * Snaps the view after a swipe, niri's `view_offset_gesture_end`: to where the projected end
     * position lines up with a column edge, focusing the column there.
     */
    function viewOffsetGestureEnd(projected) {
        if (columns.length === 0) return;
        const currentViewPos = ws.camera.x;
        const targetViewPos = projected.x;
        const gap = gaps();
        const viewWidth = viewSize().w;
        const areaFor = (col) => (sizingMode(col) === "maximized" ? parentArea() : workingArea());
        // Padding from `computeNewViewOffset`, smaller for columns about as wide as the area.
        const paddingFor = (col, area) =>
            sizingMode(col) === "maximized" ? 0 : clamp((area.w - columnWidth(col)) / 2, 0, gap);

        // Snapping points: view positions where the view lines up with column edges.
        const snaps = [];
        if (isCenteringFocusedColumn()) {
            columns.forEach((col, colIdx) => {
                const colX = columnX(colIdx);
                const colW = columnWidth(col);
                const area = areaFor(col);
                let viewPos;
                if (sizingMode(col) === "fullscreen") viewPos = colX;
                else if (area.w <= colW) viewPos = colX - area.x;
                else viewPos = colX - (area.w - colW) / 2 - area.x;
                snaps.push({ viewPos, colIdx });
            });
        } else {
            const centerOnOverflow = options("center-focused-column") === "on-overflow";
            const snapPoints = (colIdx) => {
                const col = columns[colIdx];
                const colX = columnX(colIdx);
                const colW = columnWidth(col);
                // Fullscreen columns line up with the view, others with the working area.
                if (sizingMode(col) === "fullscreen") return [colX, colX + colW];

                const area = areaFor(col);
                const leftStrut = area.x;
                const rightStrut = viewWidth - area.w - area.x;
                const padding = paddingFor(col, area);
                const center =
                    area.w <= colW ? colX - leftStrut : colX - (area.w - colW) / 2 - leftStrut;
                const isOverflowing = (adjIdx) =>
                    centerOnOverflow &&
                    adjIdx >= 0 &&
                    adjIdx < columns.length &&
                    columnWidth(columns[adjIdx]) + 3 * gap + colW > area.w;
                const left = isOverflowing(colIdx + 1) ? center : colX - padding - leftStrut;
                const right = isOverflowing(colIdx - 1)
                    ? center + viewWidth
                    : colX + colW + padding + rightStrut;
                return [left, right];
            };

            // Don't snap past the first or last column. The leftmost snap can be right of the
            // rightmost one when the columns don't fill the view, which is fine.
            const lastIdx = columns.length - 1;
            const leftmostSnap = snapPoints(0)[0];
            const rightmostSnap = snapPoints(lastIdx)[1] - viewWidth;
            snaps.push({ viewPos: leftmostSnap, colIdx: 0 });
            snaps.push({ viewPos: rightmostSnap, colIdx: lastIdx });
            for (let colIdx = 0; colIdx < columns.length; colIdx++) {
                const [left, right] = snapPoints(colIdx);
                if (leftmostSnap < left && left < rightmostSnap)
                    snaps.push({ viewPos: left, colIdx });
                const rightPos = right - viewWidth;
                if (leftmostSnap < rightPos && rightPos < rightmostSnap) {
                    snaps.push({ viewPos: rightPos, colIdx });
                }
            }
        }

        snaps.sort((a, b) => a.viewPos - b.viewPos);
        const snap = snaps[argmin(snaps, (s) => Math.abs(s.viewPos - targetViewPos))];
        let newColIdx = snap.colIdx;

        if (!isCenteringFocusedColumn()) {
            // Focus the furthest column towards the direction of the swipe that's fully visible.
            if (targetViewPos >= currentViewPos) {
                for (let colIdx = newColIdx + 1; colIdx < columns.length; colIdx++) {
                    const col = columns[colIdx];
                    const colX = columnX(colIdx);
                    const colW = columnWidth(col);
                    if (sizingMode(col) === "fullscreen") {
                        if (snap.viewPos + viewSize().w < colX + colW) break;
                    } else {
                        const area = areaFor(col);
                        if (snap.viewPos + area.x + area.w < colX + colW + paddingFor(col, area))
                            break;
                    }
                    newColIdx = colIdx;
                }
            } else {
                for (let colIdx = newColIdx - 1; colIdx >= 0; colIdx--) {
                    const col = columns[colIdx];
                    const colX = columnX(colIdx);
                    if (sizingMode(col) === "fullscreen") {
                        if (colX < snap.viewPos) break;
                    } else {
                        const area = areaFor(col);
                        if (colX - paddingFor(col, area) < snap.viewPos + area.x) break;
                    }
                    newColIdx = colIdx;
                }
            }
        }

        if (newColIdx !== activeColumnIdx) {
            viewOffsetToRestore = null;
            // niri keeps this across swipes, but it's about the column that was active before.
            activatePrevColumnOnRemoval = null;
        }
        activeColumnIdx = newColIdx;
        viewOffset = snap.viewPos - columnX(newColIdx);
        // Snapping to the right edge of a column wider than the view still shows its left edge.
        fitView();
    }

    // Dragging windows.

    /**
     * Where a window dropped with the pointer at `pointer` goes: `{ newColumn: idx }` or
     * `{ inColumn: idx, tile: idx }`, whichever gap is closest.
     */
    function insertPosition(pointer) {
        if (columns.length === 0) return { newColumn: 0 };
        // Aim for the center of the gap.
        const x = pointer.x + gaps() / 2;
        const y = pointer.y + gaps() / 2;
        // Before the first column.
        if (x < 0) return { newColumn: 0 };

        const xs = [];
        for (let idx = 0; idx <= columns.length; idx++) xs.push(columnX(idx));
        // The closest gap between columns.
        const closestColIdx = argmin(xs, (colX) => Math.abs(colX - x));
        // The column containing the position.
        let colIdx = 0;
        while (colIdx + 1 < xs.length && xs[colIdx + 1] <= x) colIdx += 1;
        // Past the last column.
        if (colIdx === columns.length) return { newColumn: closestColIdx };

        // The closest gap between tiles.
        const col = columns[colIdx];
        const offsets = tileOffsets(col);
        let closestTileIdx;
        let tileY;
        if (col.display === "tabbed") {
            // There's only one tab visible: check its top and bottom.
            const top = offsets[col.activeTileIdx].y;
            const bottom = top + col.tiles[col.activeTileIdx].size.h;
            if (Math.abs(top - y) <= Math.abs(bottom - y)) {
                [closestTileIdx, tileY] = [col.activeTileIdx, top];
            } else {
                [closestTileIdx, tileY] = [col.activeTileIdx + 1, bottom];
            }
        } else {
            closestTileIdx = argmin(offsets, (offset) => Math.abs(offset.y - y));
            tileY = offsets[closestTileIdx].y;
        }

        // The closest of the vertical and the horizontal gap.
        if (Math.abs(xs[closestColIdx] - x) <= Math.abs(tileY - y)) {
            return { newColumn: closestColIdx };
        }
        return { inColumn: colIdx, tile: closestTileIdx };
    }

    /** The highlight for an insert position, niri's `insert_hint_area`, on the canvas. */
    function insertHintArea(position) {
        const area = workingArea();
        const gap = gaps();
        let rect;
        if ("newColumn" in position) {
            const idx = position.newColumn;
            rect = { x: columnX(idx), y: area.y + gap, w: 300, h: area.h - gap * 2 };
            if (idx === 0 && columns.length > 0) {
                rect.x -= rect.w + gap;
            } else if (idx > 0 && idx < columns.length) {
                rect.x -= rect.w / 2 + gap / 2;
            }
        } else {
            const col = columns[position.inColumn];
            const tileIdx = position.tile;
            const offsets = tileOffsets(col);
            let y;
            let h = 150;
            if (col.display === "tabbed") {
                // Only one tab is visible: show the hint at its top or bottom.
                const top = offsets[col.activeTileIdx].y;
                const bottom = top + col.tiles[col.activeTileIdx].size.h;
                y = tileIdx <= col.activeTileIdx ? top : bottom - 150;
            } else {
                const top = offsets[tileIdx].y;
                if (tileIdx === 0) {
                    y = top;
                } else if (tileIdx === col.tiles.length) {
                    y = top - gap - 150;
                } else {
                    [y, h] = [top - gap / 2 - 150, 300];
                }
            }
            // Stay clear of a tab indicator placed within the column.
            const indicator =
                col.display === "tabbed" && sizingMode(col) === "normal" ? extraSize(col).w : 0;
            const x = columnX(position.inColumn) + tilesOrigin(col).x;
            rect = { x, y, w: columnWidth(col) - indicator, h };
        }

        const camera = ws.camera.x;
        if (columns.length === 0) {
            // The first window on an empty workspace drops the view offset; show the hint where
            // the window will end up on screen.
            const firstViewOffset = isCenteringFocusedColumn()
                ? computeNewViewOffsetCentered(0, 0, rect.w, "normal")
                : computeNewViewOffsetFit(0, 0, rect.w, "normal");
            rect.x += camera - firstViewOffset;
        }
        if ("newColumn" in position) {
            // Keep the hint at least partially on screen.
            rect.x = clamp(rect.x, camera - rect.w / 2, camera + viewSize().w - rect.w / 2);
        }
        return {
            x: round(rect.x),
            y: round(rect.y),
            w: round(rect.w),
            h: round(rect.h),
        };
    }

    // Saving and restoring.

    function save() {
        return {
            layout: "niri",
            version: 1,
            columns: columns.map((col) => ({
                id: col.id,
                tiles: col.tiles.map((tile) => ({
                    id: tile.win.id,
                    height: tile.height,
                    taken: tile.taken,
                })),
                activeTileIdx: col.activeTileIdx,
                width: col.width,
                presetWidthIdx: col.presetWidthIdx,
                isFullWidth: col.isFullWidth,
                pendingFullscreen: col.pendingFullscreen,
                pendingMaximized: col.pendingMaximized,
                display: col.display,
            })),
            activeColumnIdx,
            viewOffset,
            activatePrevColumnOnRemoval,
            viewOffsetToRestore,
        };
    }

    function validHeight(height) {
        if (height?.kind === "preset") {
            return height.idx < options("preset-window-heights").length
                ? height
                : { kind: "auto", weight: 1 };
        }
        return height?.kind === "auto" || height?.kind === "fixed"
            ? height
            : { kind: "auto", weight: 1 };
    }

    /** Rebuilds a column saved by `save()`. */
    function restoreColumn(saved, tiles, keepId = false) {
        const col = makeColumn(saved.width, !!saved.isFullWidth);
        if (keepId && Number.isInteger(saved.id)) {
            // Tab bars keep their keys across reloads.
            col.id = saved.id;
            nextColumnId = Math.max(nextColumnId, saved.id + 1);
        }
        col.tiles = tiles;
        col.activeTileIdx = clamp(saved.activeTileIdx ?? 0, 0, tiles.length - 1);
        const presetIdx = saved.presetWidthIdx;
        col.presetWidthIdx =
            Number.isInteger(presetIdx) && presetIdx < options("preset-column-widths").length
                ? presetIdx
                : null;
        col.pendingFullscreen =
            !!saved.pendingFullscreen && (tiles.length === 1 || saved.display === "tabbed");
        col.pendingMaximized =
            !!saved.pendingMaximized && (tiles.length === 1 || saved.display === "tabbed");
        col.display = saved.display === "tabbed" ? "tabbed" : "normal";
        if (tiles.length === 1 && tiles[0].height.kind === "auto")
            tiles[0].height = { kind: "auto", weight: 1 };
        updateTileSizes(col);
        markCommitted(col);
        return col;
    }

    function restoreState(state, windows) {
        const byId = new Map(windows.map((win) => [win.id, win]));
        let lost = false;
        let activeIdx = 0;
        state.columns.forEach((saved, idx) => {
            const tiles = [];
            for (const t of saved.tiles) {
                const win = byId.get(t.id);
                if (win === undefined) {
                    lost = true;
                    continue;
                }
                byId.delete(t.id);
                const tile = makeTile(win, validHeight(t.height));
                tile.taken = t.taken ?? null;
                tiles.push(tile);
            }
            if (idx === state.activeColumnIdx) activeIdx = columns.length;
            if (tiles.length > 0) columns.push(restoreColumn(saved, tiles, true));
            else lost = true;
        });
        if (columns.length === 0) return [...byId.values()];

        activeColumnIdx = clamp(activeIdx, 0, columns.length - 1);
        viewOffset = Number.isFinite(state.viewOffset) ? state.viewOffset : 0;
        if (!lost) {
            activatePrevColumnOnRemoval = state.activatePrevColumnOnRemoval ?? null;
            viewOffsetToRestore = state.viewOffsetToRestore ?? null;
            // Everything is back where it was, so leave the view there too.
            viewSettled = byId.size === 0;
        } else {
            fitView();
        }
        return [...byId.values()];
    }

    /**
     * Rebuilds columns from where windows are: windows that overlap horizontally were stacked in
     * one column, from top to bottom.
     */
    function columnsFromRects(windows, rects) {
        const withRect = windows
            .filter((win) => rects.get(win.id) !== null)
            .sort((a, b) => rects.get(a.id).x - rects.get(b.id).x || rects.get(a.id).y - rects.get(b.id).y);
        const groups = [];
        for (const win of withRect) {
            const rect = rects.get(win.id);
            const last = groups.at(-1);
            const alone = win.mode !== "normal";
            if (last && !alone && !last.alone && rect.x < last.right) {
                last.windows.push(win);
                last.right = Math.max(last.right, rect.x + rect.w);
            } else {
                groups.push({ windows: [win], right: rect.x + rect.w, alone });
            }
        }
        // Without a rect, there's no telling where a window was; windows usually open to the
        // right, so go by the order they opened in.
        const withoutRect = windows
            .filter((win) => rects.get(win.id) === null)
            .sort((a, b) => a.id - b.id);
        for (const win of withoutRect) groups.push({ windows: [win], alone: true });

        for (const group of groups) {
            const wins = group.windows.sort(
                (a, b) => (rects.get(a.id)?.y ?? 0) - (rects.get(b.id)?.y ?? 0),
            );
            const first = wins[0];
            const w = Math.max(...wins.map((win) => rects.get(win.id)?.w ?? win.size.w));
            const width = first.mode === "normal" ? widthForSize(w, first) : defaultWidth(first);
            const col = makeColumn(width, false);
            if (first.mode === "fullscreen") col.pendingFullscreen = true;
            if (first.mode === "maximized") col.pendingMaximized = true;
            // Keep the heights the windows had, as weights like `convertHeightsToAuto` gives.
            const heights = wins.map((win) => rects.get(win.id)?.h ?? 1);
            const median = [...heights].sort((a, b) => a - b)[Math.floor(heights.length / 2)];
            col.tiles = wins.map((win, idx) => {
                const weight = wins.length > 1 ? heights[idx] / median : 1;
                return makeTile(win, { kind: "auto", weight });
            });
            updateTileSizes(col);
            markCommitted(col);
            columns.push(col);
        }
    }

    /**
     * Takes over windows that were there when the engine started: `rects` has where each was on
     * the canvas, or `null`.
     */
    function adopt(windows, rects, focused, state) {
        let rest = windows;
        if (state?.layout === "niri" && state.version === 1 && Array.isArray(state.columns)) {
            rest = restoreState(state, windows);
            for (const win of rest) {
                addTile(
                    columns.length,
                    makeTile(win),
                    false,
                    widthForSize(rects.get(win.id)?.w ?? win.size.w, win),
                    false,
                    win.mode,
                );
            }
            if (focused !== null) activateWindow(focused);
            return;
        }

        columnsFromRects(rest, rects);
        if (columns.length === 0) return;
        const found = focused === null ? null : find(focused);
        if (found !== null) {
            [activeColumnIdx] = found;
            columns[activeColumnIdx].activeTileIdx = found[1];
        }
        // Keep the active column where it was on screen, then make sure it's in view.
        const anchorRect = rects.get(activeWindow().id);
        viewOffset =
            anchorRect === null
                ? ws.camera.x - columnX(activeColumnIdx)
                : ws.camera.x - anchorRect.x;
        fitView();
    }

    // Applying to the workspace.

    /**
     * Reacts to windows in the active column taking a new size or sizing mode, like niri does
     * when they commit (`ScrollingSpace::update_window`): remember the view offset when the column
     * goes fullscreen or maximized, give it back when it leaves, and scroll to keep it in view.
     */
    function commit(activeBefore) {
        if (columns.length > 0) {
            const col = columns[activeColumnIdx];
            const mode = sizingMode(col);
            const resized = col.tiles.some(
                (tile) => tile.committedSize === null || !sameSize(tile.size, tile.committedSize),
            );
            // A newly focused window commits too, for its activated state.
            const focusChanged = activeWindow() !== activeBefore && !viewSettled;
            if (resized || col.committedMode !== mode || focusChanged || refitView) {
                const wasNormal = col.committedMode === "normal";
                const isNormal = mode === "normal";
                if (wasNormal && !isNormal) viewOffsetToRestore = viewOffset;
                let restore = null;
                if (!wasNormal && isNormal) {
                    restore = viewOffsetToRestore;
                    viewOffsetToRestore = null;
                }
                // The view is frozen while resizing.
                if (interactiveResize === null) {
                    if (restore !== null) viewOffset = restore;
                    fitView();
                }
            }
        }
        for (const col of columns) markCommitted(col);
        refitView = false;
    }

    // The tab indicator's nodes.

    /** Whether a tabbed column in normal mode shows its tab indicator. */
    function showsIndicator(col) {
        return (
            options("tab-indicator") &&
            col.display === "tabbed" &&
            sizingMode(col) === "normal" &&
            !(options("tab-indicator-hide-when-single-tab") && col.tiles.length === 1)
        );
    }

    /** Rects of the tabs of an indicator along `area`, in order, on the canvas. */
    function tabRects(area, count) {
        const position = options("tab-indicator-position");
        const width = options("tab-indicator-width");
        const gap = options("tab-indicator-gap");
        const horizontal = position === "top" || position === "bottom";
        // The indicator spans a proportion of the side it's on, centered.
        const side = horizontal ? area.w : area.h;
        const total = side * clamp(options("tab-indicator-length"), 0, 1);
        const start = (horizontal ? area.x : area.y) + (side - total) / 2;
        const step = total / count;
        const length = count > 1 ? Math.max(0, step - TAB_SPACING) : step;
        const across = {
            left: area.x - gap - width,
            right: area.x + area.w + gap,
            top: area.y - gap - width,
            bottom: area.y + area.h + gap,
        }[position];
        return Array.from({ length: count }, (_, i) => {
            const from = start + i * step;
            return horizontal
                ? { x: round(from), y: round(across), w: length, h: width }
                : { x: round(across), y: round(from), w: width, h: length };
        });
    }

    /**
     * Draws the tab indicators of the columns in `bars`, `{ area, tiles }` by column id, and
     * takes away those of columns that don't have one.
     */
    function drawTabIndicators(bars) {
        for (const [id, bar] of bars) {
            let nodes = tabNodes.get(id);
            if (nodes === undefined) {
                nodes = [];
                tabNodes.set(id, nodes);
            }
            const count = bar.tiles.length;
            while (nodes.length > count) nodes.pop().remove();
            while (nodes.length < count) {
                const index = nodes.length;
                const node = bento.createNode("box", {
                    interactive: true,
                    transitions: { rect: "window-movement" },
                });
                node.addEventListener("click", () => clickTab(id, index));
                nodes.push(node);
                ws.tree.append(node);
            }
            const rects = tabRects(bar.area, count);
            bar.tiles.forEach((tile, idx) => {
                const color = tile.win.urgent
                    ? options("tab-indicator-urgent-color")
                    : idx === bar.active
                      ? options("tab-indicator-active-color")
                      : options("tab-indicator-inactive-color");
                nodes[idx].set({
                    rect: rects[idx],
                    background: color,
                    cornerRadius: options("tab-indicator-corner-radius"),
                });
            });
        }
        for (const [id, nodes] of tabNodes) {
            if (!bars.has(id)) {
                nodes.forEach((node) => node.remove());
                tabNodes.delete(id);
            }
        }
    }

    /** Clicking a tab activates its window. */
    function clickTab(colId, index) {
        const tile = columns.find((col) => col.id === colId)?.tiles[index];
        if (tile !== undefined) {
            mutate(() => activateWindow(tile.win), { focus: "always" });
        }
    }

    // Groups. A tabbed column of two or more windows is a group in bento, which keeps native
    // group actions and tabbed columns the same thing.

    /** Makes bento's groups match the tabbed columns: one group for each. */
    function syncGroups() {
        for (const col of columns) {
            const wins = col.tiles.map((tile) => tile.win);
            const activeWin = col.tiles[col.activeTileIdx].win;
            if (col.display !== "tabbed" || wins.length < 2) {
                // Stacked windows aren't grouped, so a group of just these windows ends.
                for (const win of wins) {
                    const group = win.group;
                    if (group !== null && group.windows.every((other) => wins.includes(other))) {
                        group.dissolve();
                    }
                }
                col.group = null;
                continue;
            }
            let group = activeWin.group;
            const matches =
                group !== null &&
                group.windows.length === wins.length &&
                wins.every((win) => win.group === group);
            if (!matches) {
                group = bento.createGroup(wins, activeWin);
            } else if (group.active !== activeWin) {
                group.activate(activeWin);
            }
            col.group = group;
        }
    }

    /**
     * Makes the strip show `group` the way it is: its windows as the tabs of one column, the one
     * of its active window, with the group's active window shown.
     */
    function matchGroup(group) {
        const found = find(group.active);
        if (found === null) return;
        const wins = group.windows.filter((win) => find(win) !== null);
        if (wins.length < 2) return;

        let col = columns[found[0]];
        col.display = "tabbed";
        const same =
            col.tiles.length === wins.length && col.tiles.every((tile) => wins.includes(tile.win));
        if (!same) {
            // Windows of the group that are elsewhere join this column as tabs.
            for (const win of wins) {
                if (col.tiles.some((tile) => tile.win === win)) continue;
                const [colIdx, tileIdx] = find(win);
                const { tile } = removeTile(colIdx, tileIdx);
                col = columns[find(group.active)[0]];
                addTileAt(col, col.tiles.length, tile);
            }
            // Windows that left the group go to columns of their own, to the right.
            for (const tile of [...col.tiles]) {
                if (wins.includes(tile.win)) continue;
                const colIdx = columns.indexOf(col);
                const removed = removeTile(colIdx, col.tiles.indexOf(tile));
                col = columns[find(group.active)[0]];
                addTile(
                    columns.indexOf(col) + 1,
                    removed.tile,
                    false,
                    removed.width,
                    removed.isFullWidth,
                    removed.mode,
                );
            }
        }
        col.group = group;
        col.activeTileIdx = col.tiles.findIndex((tile) => tile.win === group.active);
        cancelResizeForColumn(col);
        updateTileSizes(col);
    }

    /** A group went away: its column stops being tabbed, unless it was down to one tab. */
    function dropGroup(group) {
        const col = columns.find((other) => other.group === group);
        if (col === undefined) return;
        col.group = null;
        if (col.tiles.length > 1 && col.tiles.every((tile) => tile.win.group === null)) {
            col.display = "normal";
            updateTileSizes(col);
        }
    }

    // The insert hint.

    function showHint(pointer) {
        if (!options("insert-hint")) return;
        hintNode ??= bento.createNode("box", {
            cornerRadius: HINT_CORNER_RADIUS,
            transitions: { rect: "window-movement" },
        });
        hintNode.set({
            rect: insertHintArea(insertPosition(pointer)),
            background: options("insert-hint-color"),
        });
        // Last, so it's above the windows.
        ws.tree.append(hintNode);
    }

    function hideHint() {
        hintNode?.remove();
    }

    /** Places every window, draws the tab indicators, and moves the camera. */
    function apply(animate) {
        const bars = new Map();
        let x = 0;
        for (const col of columns) {
            const mode = sizingMode(col);
            const offsets = tileOffsets(col);

            // Fullscreen and maximized columns hide the tab indicator.
            if (showsIndicator(col)) {
                const origin = tilesOrigin(col);
                const active = col.tiles[col.activeTileIdx];
                bars.set(col.id, {
                    // The active tab's width, and the height of the tallest one, so the indicator
                    // stays put when switching to a smaller tab like a dialog.
                    area: {
                        x: round(x + origin.x),
                        y: round(origin.y),
                        w: active.size.w,
                        h: Math.max(...col.tiles.map((tile) => tile.size.h)),
                    },
                    tiles: col.tiles,
                    active: col.activeTileIdx,
                });
            }

            col.tiles.forEach((tile, idx) => {
                const { win } = tile;
                win.setMode(mode);
                win.setVisible(col.display !== "tabbed" || idx === col.activeTileIdx);
                const rect = {
                    x: round(x + offsets[idx].x),
                    y: round(offsets[idx].y),
                    w: tile.size.w,
                    h: tile.size.h,
                };
                win.place(rect, { animate });
            });
            x += columnWidth(col) + gaps();
        }

        drawTabIndicators(bars);
        syncGroups();

        // A swipe moves the camera itself until it ends.
        if (columns.length > 0 && !swiping) {
            const camera = { x: round(columnX(activeColumnIdx) + viewOffset), y: 0 };
            if (camera.x !== ws.camera.x || camera.y !== ws.camera.y) {
                ws.camera.move(camera, {
                    animate: animate && !skipCameraAnimation,
                    velocity: cameraVelocity,
                });
            }
        }
    }

    /**
     * Runs a call that changes the layout, then applies it.
     *
     * `focus` says when to focus the active window afterwards: `"always"`, `"changed"` when it
     * changed, or `"follow"` when it changed and the engine had focus, so that closing a window
     * doesn't take focus from somewhere else.
     */
    function mutate(fn, { animate = true, focus = "follow", hadFocus = ws.focused !== null } = {}) {
        // Swipes and dragging near the view's edges move the camera without the strip; take the
        // view offset from wherever it is.
        if (columns.length > 0) viewOffset = ws.camera.x - columnX(activeColumnIdx);
        const activeBefore = activeWindow();
        skipCameraAnimation = false;
        viewSettled = false;

        const result = fn();

        commit(activeBefore);
        apply(animate);

        const active = activeWindow();
        const changed = active !== activeBefore;
        if (active !== null && (focus === "always" || (changed && (focus === "changed" || hadFocus)))) {
            if (ws.focused !== active) active.focus();
            active.raise();
        }
        lastFocused = ws.focused;

        const exposedNow = JSON.stringify(exposed());
        if (exposedNow !== lastExposed) {
            lastExposed = exposedNow;
            bento.stateChanged();
        }
        return result;
    }

    /** The strip's state for those reading it from outside. */
    function exposed() {
        return {
            active: columns.length > 0 ? activeColumnIdx : null,
            columns: columns.map((col) => ({
                windows: col.tiles.map((tile) => tile.win.id),
                active: col.tiles[col.activeTileIdx].win.id,
                width: col.width,
                fullWidth: col.isFullWidth,
                display: col.display,
                mode: sizingMode(col),
            })),
        };
    }

    // Actions.

    function targetColumn(target) {
        if (target === null) return [columns[activeColumnIdx], null];
        const [colIdx, tileIdx] = find(target);
        return [columns[colIdx], tileIdx];
    }

    /** Arguments of an action: indices, size changes, display modes and `forwards` flags. */
    const index = (args) => (Number.isFinite(args[0]) ? Math.trunc(args[0]) : 1);
    const forwards = (name, args) => !name.endsWith("-back") && args[0] !== false;

    function sizeChange(arg) {
        if (arg === null || typeof arg !== "object") return null;
        if (!["set", "adjust"].includes(arg.op) || !["px", "%"].includes(arg.unit)) return null;
        return Number.isFinite(arg.value) ? arg : null;
    }

    function runAction(name, args, target) {
        if (target !== null && find(target) === null) return;
        if (columns.length === 0) return;
        const col = columns[activeColumnIdx];

        switch (name) {
            case "focus-column-left":
                return focusLeft();
            case "focus-column-right":
                return focusRight();
            case "focus-column-first":
                return focusColumnFirst();
            case "focus-column-last":
                return focusColumnLast();
            case "focus-column-right-or-first":
                if (!focusRight()) focusColumnFirst();
                return;
            case "focus-column-left-or-last":
                if (!focusLeft()) focusColumnLast();
                return;
            case "focus-column":
                return activateColumn(clamp(index(args) - 1, 0, columns.length - 1));
            case "focus-window-in-column":
                return activateTile(col, clamp(index(args) - 1, 0, col.tiles.length - 1));
            case "focus-window-down":
                return focusDown();
            case "focus-window-up":
                return focusUp();
            case "focus-window-down-or-column-left":
                if (!focusDown()) focusLeft();
                return;
            case "focus-window-down-or-column-right":
                if (!focusDown()) focusRight();
                return;
            case "focus-window-up-or-column-left":
                if (!focusUp()) focusLeft();
                return;
            case "focus-window-up-or-column-right":
                if (!focusUp()) focusRight();
                return;
            case "focus-window-top":
                return focusTop();
            case "focus-window-bottom":
                return focusBottom();
            case "focus-window-down-or-top":
                if (!focusDown()) focusTop();
                return;
            case "focus-window-up-or-bottom":
                if (!focusUp()) focusBottom();
                return;

            case "move-column-left":
                return moveLeft();
            case "move-column-right":
                return moveRight();
            case "move-column-to-first":
                return moveColumnTo(0);
            case "move-column-to-last":
                return moveColumnTo(columns.length - 1);
            case "move-column-to-index":
                return moveColumnTo(clamp(index(args) - 1, 0, columns.length - 1));
            case "move-window-down":
                return moveInColumn(1);
            case "move-window-up":
                return moveInColumn(-1);

            case "consume-or-expel-window-left":
                return consumeOrExpelWindowLeft(target);
            case "consume-or-expel-window-right":
                return consumeOrExpelWindowRight(target);
            case "consume-window-into-column":
                return consumeIntoColumn();
            case "expel-window-from-column":
                return expelFromColumn();
            case "swap-window-left":
                return swapWindowInDirection("left");
            case "swap-window-right":
                return swapWindowInDirection("right");

            case "toggle-column-tabbed-display":
                return setColumnDisplay(col.display === "tabbed" ? "normal" : "tabbed");
            case "set-column-display":
                if (args[0] === "normal" || args[0] === "tabbed") setColumnDisplay(args[0]);
                return;

            case "center-column":
                return centerColumn();
            case "center-window":
                return centerWindow(target);
            case "center-visible-columns":
                return centerVisibleColumns();

            case "switch-preset-column-width":
            case "switch-preset-column-width-back":
                toggleWidth(col, null, forwards(name, args));
                return cancelResizeForColumn(col);
            case "switch-preset-window-width":
            case "switch-preset-window-width-back": {
                const [targetCol, tileIdx] = targetColumn(target);
                toggleWidth(targetCol, tileIdx, forwards(name, args));
                return cancelResizeForColumn(targetCol);
            }
            case "switch-preset-window-height":
            case "switch-preset-window-height-back": {
                const [targetCol, tileIdx] = targetColumn(target);
                toggleWindowHeight(targetCol, tileIdx, forwards(name, args));
                return cancelResizeForColumn(targetCol);
            }
            case "maximize-column":
                toggleFullWidth(col);
                return cancelResizeForColumn(col);
            case "set-column-width":
            case "set-window-width": {
                const change = sizeChange(args[0]);
                if (change === null) return;
                const [targetCol, tileIdx] =
                    name === "set-column-width" ? [col, null] : targetColumn(target);
                setColumnWidth(targetCol, change, tileIdx);
                return cancelResizeForColumn(targetCol);
            }
            case "set-window-height": {
                const change = sizeChange(args[0]);
                if (change === null) return;
                const [targetCol, tileIdx] = targetColumn(target);
                setWindowHeight(targetCol, change, tileIdx);
                return cancelResizeForColumn(targetCol);
            }
            case "reset-window-height": {
                const [targetCol, tileIdx] = targetColumn(target);
                resetWindowHeight(targetCol, tileIdx);
                return cancelResizeForColumn(targetCol);
            }
            case "expand-column-to-available-width":
                return expandColumnToAvailableWidth();
        }
    }

    // The strip.

    /** The size a window opening on this workspace gets, niri's `new_window_size`. */
    function newWindowSize({ rules, minSize, maxSize, borders }) {
        const area = workingArea();
        const extra = newColumnExtraSize();
        const fromPreset = (preset, length, extra, border) => {
            const resolved = resolvePresetSize(preset, length, extra);
            return resolved.tile !== undefined ? resolved.tile - border : resolved.window;
        };
        const width = rulesWidth(rules);
        const height = rulesHeight(rules);
        let w =
            width === null
                ? 0
                : Math.max(1, Math.floor(fromPreset(width, area.w, extra.w, borders.w)));
        const fullHeight = area.h - gaps() * 2 - borders.h;
        let h =
            height === null
                ? fullHeight
                : Math.min(fromPreset(height, area.h, extra.h, borders.h), fullHeight);
        h = Math.max(Math.floor(h), 1);

        // Window limits, without borders.
        const limit = (value, border) => (value > 0 ? Math.max(1, value - border) : 0);
        const min = { w: limit(minSize.w, borders.w), h: limit(minSize.h, borders.h) };
        const max = { w: limit(maxSize.w, borders.w), h: limit(maxSize.h, borders.h) };
        // Fixed-size windows get their size, others can pick a width if none is set.
        if (w !== 0) w = applyLimits(w, min.w, max.w);
        else if (min.w > 0 && min.w === max.w) w = min.w;
        // Scrolling only honors a fixed height, and always min height.
        if (min.h === max.h) h = applyLimits(h, min.h, max.h);
        else h = Math.max(h, min.h);

        return { w: w === 0 ? 0 : w + borders.w, h: h + borders.h };
    }

    /** Resets what a window that moved to another strip remembers of the one it left. */
    function resetTile(tile) {
        tile.taken = null;
        tile.seen = { size: { ...tile.win.size }, limits: limitsKey(tile.win) };
        tile.committedSize = null;
    }

    return {
        has: (win) => find(win) !== null,
        exposed,

        windowAdded(win, how) {
            hideHint();
            mutate(() => addWindow(win, how), { focus: how.activate ? "always" : "follow" });
        },

        windowRemoved(win, reason) {
            const hadFocus = ws.focused !== null || lastFocused?.id === win.id;
            mutate(() => removeWindow(win, reason), { hadFocus });
        },

        /** Takes over the windows that were here when the engine started, `{ win, rect }`. */
        adopt(entries, focused, state) {
            const rects = new Map(entries.map(({ win, rect }) => [win.id, rect ?? null]));
            mutate(
                () => {
                    adopt(
                        entries.map(({ win }) => win),
                        rects,
                        focused,
                        state,
                    );
                    // Groups that were already there show as tabbed columns.
                    for (const group of bento.groups) {
                        if (find(group.active) !== null) matchGroup(group);
                    }
                },
                { focus: focused !== null ? "always" : "follow" },
            );
        },

        save,

        /** Runs an action; returns whether it changed the layout. */
        action(name, args, target) {
            return mutate(
                () => {
                    const before = JSON.stringify(save());
                    runAction(name, args ?? [], target);
                    return JSON.stringify(save()) !== before;
                },
                { focus: "changed" },
            );
        },

        /** Gives focus to the active window, for a strip that just became the one shown. */
        focusActive() {
            mutate(() => {}, { focus: "always" });
        },

        activate(win) {
            mutate(() => activateWindow(win), { focus: "always" });
        },

        modeRequested(win, mode, request) {
            mutate(() => {
                const found = find(win);
                if (found === null) return;
                const col = columns[found[0]];
                if (request?.fullscreen !== undefined) {
                    // Leaving fullscreen goes back to maximized if the window was.
                    setFullscreen(win, request.fullscreen);
                } else if (request?.maximized !== undefined) {
                    // A fullscreen window stays fullscreen, and leaving it goes to maximized.
                    setMaximized(win, request.maximized);
                } else if (mode === "fullscreen") {
                    setFullscreen(win, true);
                } else if (mode === "maximized") {
                    setMaximized(win, true);
                } else if (col.pendingFullscreen) {
                    setFullscreen(win, false);
                } else {
                    setMaximized(win, false);
                }
            });
        },

        windowChanged(win) {
            mutate(() => {
                const found = find(win);
                if (found === null) return;
                const [colIdx, tileIdx] = found;
                const col = columns[colIdx];
                const tile = col.tiles[tileIdx];
                const limits = limitsKey(win);
                if (limits !== tile.seen.limits) {
                    // New limits may let the window take the size it couldn't before.
                    tile.taken = null;
                }
                if (!sameSize(win.size, tile.seen.size)) {
                    // The window took a size: if it isn't the one it was asked for, lay out from
                    // the size it took, as long as it's asked for the same.
                    const refused =
                        win.size.w > 0 && win.size.h > 0 && !sameSize(win.size, tile.size);
                    tile.taken = refused ? { ...win.size, for: sizeKey(tile.request) } : null;
                }
                tile.seen = { size: { ...win.size }, limits };
                updateTileSizes(col);
                // niri scrolls to keep the active column in view whenever its windows change.
                if (colIdx === activeColumnIdx) refitView = true;
            });
        },

        /** The monitor's size, scale or working area changed, or the workspace moved to another. */
        viewChanged() {
            mutate(() => {
                updateAllTileSizes();
                if (columns.length > 0) fitView();
            });
        },

        configChanged() {
            mutate(() => {
                const now = bento.options;
                if (!samePresets(now["preset-column-widths"], lastOptions["preset-column-widths"])) {
                    for (const col of columns) col.presetWidthIdx = null;
                }
                if (!samePresets(now["preset-window-heights"], lastOptions["preset-window-heights"])) {
                    for (const col of columns) convertHeightsToAuto(col);
                }
                lastOptions = now;
                updateAllTileSizes();
                // Apply always-center and such right away.
                if (columns.length > 0) fitView();
            });
        },

        // Groups.

        groupChanged(group) {
            mutate(() => matchGroup(group));
        },

        groupRemoved(group) {
            mutate(() => dropGroup(group));
        },

        // Pointer resizing.

        resizeBegin(win, resize) {
            if (interactiveResize !== null) return false;
            const found = find(win);
            if (found === null) return false;
            const col = columns[found[0]];
            if (sizingMode(col) !== "normal") return false;
            const tile = col.tiles[found[1]];
            interactiveResize = {
                win,
                edges: { ...resize.edges },
                originalWindowSize: {
                    w: windowWidthForTileWidth(tile, tile.size.w),
                    h: windowHeightForTileHeight(tile, tile.size.h),
                },
            };
            return true;
        },

        resizeUpdate(win, resize) {
            mutate(() => interactiveResizeUpdate(win, resize.delta), { animate: false });
        },

        resizeEnd(win) {
            mutate(() => {
                if (interactiveResize?.win !== win) return;
                interactiveResize = null;
                // Scroll the resized window into view right away.
                const found = find(win);
                if (found !== null && found[0] === activeColumnIdx) fitView();
            });
        },

        /** A double click on a resize edge: left and right toggle the width, top and bottom the height. */
        edgeDoubleClick(win, edges) {
            mutate(() => {
                const found = find(win);
                if (found === null) return;
                const [colIdx, tileIdx] = found;
                const col = columns[colIdx];
                if (sizingMode(col) === "fullscreen") return;
                if (edges.left || edges.right) toggleFullWidth(col);
                if (edges.top || edges.bottom) {
                    if (col.tiles[tileIdx].height.kind === "auto") {
                        setWindowHeight(col, { op: "set", value: 100, unit: "%" }, tileIdx);
                    } else {
                        resetWindowHeight(col, tileIdx);
                    }
                }
                cancelResizeForColumn(col);
            });
        },

        // Swipes. The camera follows the fingers, and the end snaps it to a column.

        swipeBegin() {
            swiping = true;
        },

        swipeUpdate(delta) {
            ws.camera.move({ x: ws.camera.x + delta }, { animate: false });
        },

        swipeEnd(velocity) {
            swiping = false;
            const projected = ws.camera.x - velocity / (1000 * Math.log(SWIPE_DECELERATION));
            cameraVelocity = velocity;
            try {
                mutate(() => viewOffsetGestureEnd({ x: projected, y: 0 }), { focus: "changed" });
            } finally {
                cameraVelocity = 0;
            }
        },

        // Dragging windows over the workspace.

        dragOver: showHint,
        dragLeave: hideHint,

        newWindowSize,

        // Moving between workspaces.

        /** Takes the column of `target`, or the active one, out for another workspace. */
        takeColumn(target) {
            return mutate(() => {
                if (columns.length === 0) return null;
                const colIdx = target === null ? activeColumnIdx : find(target)[0];
                const col = columns[colIdx];
                const windows = col.tiles.map((tile) => tile.win);
                // The group is made again where the column lands.
                col.group?.dissolve();
                col.group = null;
                removeColumn(colIdx);
                return { column: col, windows };
            });
        },

        /** Puts a column that came from another workspace right of the active one. */
        putColumn(col, activate) {
            mutate(
                () => {
                    col.id = nextColumnId++;
                    col.tiles.forEach(resetTile);
                    updateTileSizes(col);
                    markCommitted(col);
                    addColumn(null, col, activate);
                },
                { focus: activate ? "always" : "follow" },
            );
        },

        /** Takes the window of `target`, or the active one, out of its column. */
        takeTile(target) {
            return mutate(() => {
                if (columns.length === 0) return null;
                const [colIdx, tileIdx] = target === null ? [activeColumnIdx, columns[activeColumnIdx].activeTileIdx] : find(target);
                const col = columns[colIdx];
                col.group?.remove(col.tiles[tileIdx].win);
                return removeTile(colIdx, tileIdx);
            });
        },

        /** Puts a window that came from another workspace in a column of its own. */
        putTile(removed, activate) {
            mutate(
                () => {
                    resetTile(removed.tile);
                    addTile(null, removed.tile, activate, removed.width, removed.isFullWidth, removed.mode);
                },
                { focus: activate ? "always" : "follow" },
            );
        },

        /** Takes away what the strip drew, for a workspace that's gone. */
        dispose() {
            for (const nodes of tabNodes.values()) nodes.forEach((node) => node.remove());
            tabNodes.clear();
            hideHint();
        },
    };
}

// The engine: one strip for each workspace, and what crosses from one to another.

export default function create(bento) {
    /** Strips by workspace id. */
    const strips = new Map();
    /** Windows and groups whose events are followed. */
    const followed = new WeakSet();
    /**
     * Windows that were there when the engine started, by workspace: they come to the engine one
     * by one, and a strip takes them over together, once the last one arrived.
     */
    const adopting = new Map();

    const geometryOf = (ws) =>
        JSON.stringify([ws.monitor?.name, ws.monitor?.size, ws.monitor?.scale, ws.monitor?.workingArea]);

    function stripOf(ws) {
        let strip = strips.get(ws.id);
        if (strip === undefined) {
            strip = createStrip(bento, ws);
            strips.set(ws.id, strip);
            watchWorkspace(ws, strip);
        }
        return strip;
    }

    /** The strip a window is in, if the engine places it. */
    function stripWith(win) {
        const strip = strips.get(win.workspace?.id);
        return strip?.has(win) ? strip : undefined;
    }

    function watchWorkspace(ws, strip) {
        let geometry = geometryOf(ws);
        ws.addEventListener("change", () => {
            // Moving to another monitor lays the columns out again; switching to it doesn't.
            const now = geometryOf(ws);
            if (now !== geometry) {
                geometry = now;
                strip.viewChanged();
            }
        });
        ws.addEventListener("remove", () => {
            strip.dispose();
            strips.delete(ws.id);
            bento.stateChanged();
        });
        ws.addEventListener("swipebegin", () => strip.swipeBegin());
        ws.addEventListener("swipeupdate", (e) => strip.swipeUpdate(e.delta));
        ws.addEventListener("swipeend", (e) => strip.swipeEnd(e.velocity));
        ws.addEventListener("dragenter", (e) => strip.dragOver(e.pointer));
        ws.addEventListener("dragmove", (e) => strip.dragOver(e.pointer));
        ws.addEventListener("dragleave", () => strip.dragLeave());
    }

    function watchMonitor(monitor) {
        monitor.addEventListener("change", () => {
            for (const ws of monitor.workspaces) strips.get(ws.id)?.viewChanged();
        });
    }

    function watchGroup(group) {
        if (followed.has(group)) return;
        followed.add(group);
        group.addEventListener("change", () => matchGroup(group));
        group.addEventListener("remove", () => {
            for (const strip of strips.values()) strip.groupRemoved(group);
        });
    }

    function matchGroup(group) {
        stripWith(group.active)?.groupChanged(group);
    }

    function watchWindow(win) {
        if (followed.has(win)) return;
        followed.add(win);
        win.addEventListener("remove", (e) => stripWith(win)?.windowRemoved(win, e.reason));
        win.addEventListener("move", (e) => {
            // bento moved the window with one of its own actions.
            const from = strips.get(e.from.id);
            if (from?.has(win)) {
                from.windowRemoved(win, "move");
                stripOf(e.to).windowAdded(win, {
                    reason: "move",
                    activate: e.activate,
                    rect: null,
                    pointer: null,
                    nextTo: null,
                });
            }
        });
        win.addEventListener("change", () => stripWith(win)?.windowChanged(win));
        win.addEventListener("moderequest", (e) => {
            const strip = stripWith(win);
            if (strip !== undefined) {
                e.preventDefault();
                strip.modeRequested(win, e.mode, { ...e.request });
            }
        });
        win.addEventListener("activaterequest", () => stripWith(win)?.activate(win));
        win.addEventListener("resizebegin", (e) => {
            // Without the engine's say, bento refuses the resize.
            const strip = stripWith(win);
            if (strip === undefined || !strip.resizeBegin(win, e)) e.preventDefault();
        });
        win.addEventListener("resizeupdate", (e) => stripWith(win)?.resizeUpdate(win, e));
        win.addEventListener("resizeend", () => stripWith(win)?.resizeEnd(win));
        win.addEventListener("edgedoubleclick", (e) => stripWith(win)?.edgeDoubleClick(win, e.edges));
    }

    // Across strips.

    /** Moves keyboard focus to a monitor. Returns whether there was one. */
    function focusMonitor(monitor) {
        if (monitor === null || monitor === undefined) return false;
        monitor.focus();
        const ws = monitor.activeWorkspace;
        if (ws.focused === null) stripOf(ws).focusActive();
        return true;
    }

    /** Shows a workspace, and focuses its window. Returns whether there was one. */
    function focusWorkspace(ws) {
        if (ws === null || ws === undefined) return false;
        ws.show();
        ws.monitor.focus();
        if (ws.focused === null) stripOf(ws).focusActive();
        return true;
    }

    const below = (ws) => ws.monitor.workspaces[ws.index + 1];
    const above = (ws) => ws.monitor.workspaces[ws.index - 1];

    /** The monitor `step` places from the workspace's monitor, going around. */
    function monitorBy(ws, step) {
        const monitors = bento.monitors;
        return monitors[(monitors.indexOf(ws.monitor) + step + monitors.length) % monitors.length];
    }

    /** Moves the column of `target` to a workspace, and focus goes along with it. */
    function moveColumn(strip, target, to, focus) {
        if (to === null || to === undefined) return false;
        const taken = strip.takeColumn(target);
        if (taken === null) return false;
        for (const win of taken.windows) win.moveTo(to);
        stripOf(to).putColumn(taken.column, focus);
        if (focus) focusWorkspace(to);
        return true;
    }

    /** Moves a window to a workspace, as a column of its own. */
    function moveWindow(strip, target, to, focus) {
        if (to === null || to === undefined) return false;
        const removed = strip.takeTile(target);
        if (removed === null) return false;
        removed.tile.win.moveTo(to);
        stripOf(to).putTile(removed, focus);
        if (focus) focusWorkspace(to);
        return true;
    }

    /**
     * Actions that go on to another monitor or workspace when the strip can't do them; the
     * others are the strip's own. Each gets the strip, the window it's for, and the arguments.
     */
    const across = {
        "focus-column-or-monitor-left": (ws, strip, target) =>
            strip.action("focus-column-left", [], target) || focusMonitor(ws.monitor.left),
        "focus-column-or-monitor-right": (ws, strip, target) =>
            strip.action("focus-column-right", [], target) || focusMonitor(ws.monitor.right),
        "focus-window-or-monitor-up": (ws, strip, target) =>
            strip.action("focus-window-up", [], target) || focusMonitor(ws.monitor.up),
        "focus-window-or-monitor-down": (ws, strip, target) =>
            strip.action("focus-window-down", [], target) || focusMonitor(ws.monitor.down),
        "focus-window-or-workspace-up": (ws, strip, target) =>
            strip.action("focus-window-up", [], target) || focusWorkspace(above(ws)),
        "focus-window-or-workspace-down": (ws, strip, target) =>
            strip.action("focus-window-down", [], target) || focusWorkspace(below(ws)),

        "move-column-left-or-to-monitor-left": (ws, strip, target, args) =>
            strip.action("move-column-left", [], target) ||
            moveColumn(strip, target, ws.monitor.left?.activeWorkspace, args[0] !== false),
        "move-column-right-or-to-monitor-right": (ws, strip, target, args) =>
            strip.action("move-column-right", [], target) ||
            moveColumn(strip, target, ws.monitor.right?.activeWorkspace, args[0] !== false),

        "move-column-to-workspace": (ws, strip, target, args) =>
            moveColumn(strip, target, args[0], args[1] !== false),
        "move-column-to-workspace-up": (ws, strip, target, args) =>
            moveColumn(strip, target, above(ws), args[0] !== false),
        "move-column-to-workspace-down": (ws, strip, target, args) =>
            moveColumn(strip, target, below(ws), args[0] !== false),
        "move-column-to-monitor": (ws, strip, target, args) =>
            moveColumn(strip, target, args[0]?.activeWorkspace, args[1] !== false),
        "move-column-to-monitor-left": (ws, strip, target, args) =>
            moveColumn(strip, target, ws.monitor.left?.activeWorkspace, args[0] !== false),
        "move-column-to-monitor-right": (ws, strip, target, args) =>
            moveColumn(strip, target, ws.monitor.right?.activeWorkspace, args[0] !== false),
        "move-column-to-monitor-up": (ws, strip, target, args) =>
            moveColumn(strip, target, ws.monitor.up?.activeWorkspace, args[0] !== false),
        "move-column-to-monitor-down": (ws, strip, target, args) =>
            moveColumn(strip, target, ws.monitor.down?.activeWorkspace, args[0] !== false),
        "move-column-to-monitor-previous": (ws, strip, target, args) =>
            moveColumn(strip, target, monitorBy(ws, -1).activeWorkspace, args[0] !== false),
        "move-column-to-monitor-next": (ws, strip, target, args) =>
            moveColumn(strip, target, monitorBy(ws, 1).activeWorkspace, args[0] !== false),

        "move-window-down-or-to-workspace-down": (ws, strip, target, args) =>
            strip.action("move-window-down", [], target) ||
            moveWindow(strip, target, below(ws), args[0] !== false),
        "move-window-up-or-to-workspace-up": (ws, strip, target, args) =>
            strip.action("move-window-up", [], target) ||
            moveWindow(strip, target, above(ws), args[0] !== false),
    };

    // Listening.

    bento.addEventListener("monitoradd", (e) => watchMonitor(e.monitor));
    bento.addEventListener("workspaceadd", (e) => stripOf(e.workspace));
    bento.addEventListener("groupadd", (e) => {
        watchGroup(e.group);
        matchGroup(e.group);
    });
    bento.addEventListener("configchange", () => {
        for (const strip of strips.values()) strip.configChanged();
    });
    bento.addEventListener("newwindowsize", (e) => e.respondWith(stripOf(e.workspace).newWindowSize(e)));

    bento.addEventListener("windowadd", (e) => {
        const win = e.window;
        watchWindow(win);
        const ws = win.workspace;
        const pending = adopting.get(ws.id);
        if (e.reason === "adopt" && pending !== undefined) {
            pending.entries.push({ win, rect: e.rect, activate: e.activate });
            if (pending.entries.length === pending.expected) {
                adopting.delete(ws.id);
                const state = bento.savedState?.workspaces?.[ws.id] ?? null;
                const focused = pending.entries.find((entry) => entry.activate)?.win ?? null;
                stripOf(ws).adopt(pending.entries, focused, state);
            }
            return;
        }
        stripOf(ws).windowAdded(win, {
            reason: e.reason,
            activate: e.activate,
            rect: e.rect,
            pointer: e.pointer,
            nextTo: e.nextTo,
        });
    });

    bento.addEventListener("action", (e) => {
        const ws = e.window?.workspace ?? bento.focusedMonitor?.activeWorkspace;
        if (ws === null || ws === undefined) return;
        const strip = stripOf(ws);
        // An action on a window the engine doesn't place, like a floating one, is for the strip.
        const target = e.window !== null && strip.has(e.window) ? e.window : null;
        const args = [...e.args];
        if (e.name in across) {
            across[e.name](ws, strip, target, args);
        } else {
            strip.action(e.name, args, target);
        }
    });

    bento.addEventListener("unload", (e) => {
        e.saveState({
            workspaces: Object.fromEntries([...strips].map(([id, strip]) => [id, strip.save()])),
        });
    });

    bento.exposeState(() => ({
        workspaces: [...strips].map(([id, strip]) => ({ id, ...strip.exposed() })),
    }));

    // What's there already.

    for (const monitor of bento.monitors) {
        watchMonitor(monitor);
        for (const ws of monitor.workspaces) {
            stripOf(ws);
            if (ws.windows.length > 0) {
                adopting.set(ws.id, { expected: ws.windows.length, entries: [] });
            }
        }
    }
    for (const group of bento.groups) watchGroup(group);
}
