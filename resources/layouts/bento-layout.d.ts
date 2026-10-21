// Types for bento tiling layouts, API version 1.
//
// A layout is a single ES module that bento loads from the `tiling` section of its config:
//
//     tiling {
//         layout "bento"              // built in; also "niri", or a path to a .js file
//         options {
//             gravity true            // options that the layout declares in `meta.options`
//         }
//     }
//
// The module exports `meta`, describing the layout, and a default function that creates the layout
// for one workspace:
//
//     export const meta = { api: 1, name: "my-layout", actions: { ... }, options: { ... } };
//     export default function create(ws) { return { windowAdded(win, how) { ... }, ... }; }
//
// bento owns windows, rendering, animations, and the camera's motion. The layout owns everything
// else: where windows go, which window has focus, where the camera points, and what actions do.
//
// Coordinates are logical pixels on the workspace canvas, which is unbounded in every direction.
// The camera is the top-left corner of the visible view on the canvas. Window rects are tile rects:
// they include borders, see `Window.borders`.
//
// Layouts run in a sandbox: there is no file, network or module access, and every call has a time
// and memory limit. Calls are synchronous, and bento applies what a call did when it returns.

export interface Point {
    x: number;
    y: number;
}

export interface Size {
    w: number;
    h: number;
}

export interface Rect {
    x: number;
    y: number;
    w: number;
    h: number;
}

/**
 * A size from the config or window rules: a proportion of the working area, or fixed pixels. Like
 * in niri, a fixed size is for the window itself, without its borders.
 */
export type Preset = { proportion: number } | { fixed: number };

/** How a window is sized, as the window is told. */
export type SizingMode = "normal" | "fullscreen" | "maximized";

/**
 * A size change from an action, like `set-column-width "+10%"`.
 *
 * `"%"` values are percentages of the working area, like in the config.
 */
export interface SizeChange {
    op: "set" | "adjust";
    value: number;
    unit: "px" | "%";
}

export interface WindowRules {
    /** `default-column-width` from the window rules or the layout config; `null` if unset. */
    defaultWidth: Preset | null;
    /** `default-window-height` from the window rules; `null` if unset. */
    defaultHeight: Preset | null;
    /** `default-column-display` from the window rules or the layout config. */
    defaultColumnDisplay: "normal" | "tabbed";
    /** Whether the window opens as wide as the view, from `open-maximized`. */
    openFullWidth: boolean;
}

/** A tiled window. Objects stay the same for the lifetime of the window on a workspace. */
export interface Window {
    readonly id: number;
    readonly appId: string | null;
    readonly title: string | null;
    /** Tile size the window has right now. Windows don't always take the size they're given. */
    readonly size: Size;
    /** Smallest tile size the window accepts; 0 means no limit. */
    readonly minSize: Size;
    /** Largest tile size the window accepts; 0 means no limit. */
    readonly maxSize: Size;
    /** Total width and height of the borders around the window, included in tile sizes. */
    readonly borders: Size;
    /** Sizing mode the window was last told to take. */
    readonly mode: SizingMode;
    readonly rules: WindowRules;
    readonly isUrgent: boolean;
    /**
     * Where the layout last placed the window, or `null` if it hasn't yet. In `adopt`, where the
     * window was before the layout took over.
     */
    readonly rect: Rect | null;
    /** Whether the window is shown. */
    readonly visible: boolean;
    /**
     * What this layout stored on the window with `Workspace.setWindowData`, or `null`. It stays
     * with the window when it moves to another workspace, floats and comes back, or is dragged,
     * as long as the same layout gets it.
     */
    readonly data: unknown;
}

export interface View {
    /** Size of the output. */
    readonly size: Size;
    /** Part of the view not covered by panels and other exclusive layer surfaces. */
    readonly workingArea: Rect;
    /**
     * `workingArea` minus the struts from the config, starting at a physical pixel: where tiled
     * windows go, if the engine follows the config's struts.
     */
    readonly tilingArea: Rect;
    /** Output scale. Place windows on physical pixels to keep them sharp, see `Workspace.round`. */
    readonly scale: number;
}

/** The `layout {}` section of the config, which layouts may follow or ignore. */
export interface LayoutConfig {
    readonly gaps: number;
    /** Space to leave around tiled windows, already taken out of `View.tilingArea`. */
    readonly struts: { readonly left: number; readonly right: number; readonly top: number; readonly bottom: number };
    readonly centerFocusedColumn: "never" | "always" | "on-overflow";
    readonly alwaysCenterSingleColumn: boolean;
    readonly defaultColumnWidth: Preset | null;
    readonly presetColumnWidths: readonly Preset[];
    readonly presetWindowHeights: readonly Preset[];
    readonly defaultColumnDisplay: "normal" | "tabbed";
    /** The tab indicator that bento draws for tab bars, see `Workspace.setTabBar`. */
    readonly tabIndicator: TabIndicatorConfig;
}

export interface TabIndicatorConfig {
    readonly off: boolean;
    readonly position: "left" | "right" | "top" | "bottom";
    /** Thickness of the indicator. */
    readonly width: number;
    /** Space between the indicator and the tabs; negative to overlap them. */
    readonly gap: number;
    /**
     * Whether the indicator goes inside the tabbed area, so the windows should leave
     * `width + gap` free on the indicator's side, see `Workspace.tabIndicatorSpace`.
     */
    readonly placeWithinColumn: boolean;
    readonly hideWhenSingleTab: boolean;
}

/** Windows shown as tabs in one area, see `Workspace.setTabBar`. */
export interface TabBar {
    /** Where the tabs are; bento draws the indicator along this rect, as the config says. */
    area: Rect;
    /** The tabs, in order. */
    windows: Window[];
    /** The tab that's shown. */
    active: Window;
}

export type OptionValue = boolean | number | string;

/** Options a layout accepts in the `tiling { options {} }` section of the config. */
export interface OptionSpec {
    type: "boolean" | "number" | "string";
    default: OptionValue;
    description?: string;
}

/** An action a layout provides, to bind as `layout-action "name" args...`. */
export interface ActionSpec {
    description?: string;
    /** Names of the arguments, for documentation. */
    args?: string[];
}

export interface Meta {
    /** API version the layout is written for. */
    api: 1;
    name: string;
    description?: string;
    /**
     * Actions the layout provides on top of niri's built-in ones.
     *
     * niri's built-in layout actions, like `focus-column-left` or `set-column-width`, go to every
     * engine under their own names, so an engine can give them its own meaning. That includes
     * actions that combine others: for `focus-column-or-monitor-left`, the engine decides what
     * happens within the workspace, and if `Layout.action` returns false, bento focuses the
     * monitor to the left. Fullscreen and maximize actions arrive as `Layout.modeRequested`.
     */
    actions?: Record<string, ActionSpec>;
    options?: Record<string, OptionSpec>;
}

/** Why a window is added to the layout. */
export type AddReason =
    /** A new window opened. */
    | "open"
    /** The window was on the workspace when the layout took over, see `Layout.adopt`. */
    | "adopt"
    /** The window was moved here from another workspace or output. */
    | "move"
    /** The window was dropped here at the end of a drag. */
    | "drop"
    /** The window stopped floating. */
    | "tile";

export interface AddInfo {
    reason: AddReason;
    /** Whether the window should get focus. */
    activate: boolean;
    /**
     * Where the window was on this canvas: before the layout took over, while floating, or where a
     * dragged window was dropped. A hint, which the layout is free to ignore.
     */
    rect: Rect | null;
    /** Pointer position on the canvas, when the window was dropped. */
    pointer: Point | null;
    /** For a new window, the window it's related to, like a dialog's parent. */
    nextTo: Window | null;
}

/** Why a window is removed from the layout. */
export type RemoveReason = "close" | "move" | "drag" | "float";

/** Windows that move to another workspace together, see `Layout.transferOut`. */
export interface Transfer {
    windows: Window[];
    /** Anything JSON-serializable, given to the layout on the other workspace. */
    data?: unknown;
}

/** A pointer drag resizing a window. */
export interface Resize {
    edges: { left: boolean; right: boolean; top: boolean; bottom: boolean };
    /** How far the pointer has moved since the drag started. */
    delta: Point;
}

/** The workspace a layout instance manages, and everything the layout can do to it. */
export interface Workspace {
    /** Windows in the layout, bottom to top. */
    readonly windows: readonly Window[];
    readonly focused: Window | null;
    readonly view: View;
    /** Where the camera is going, if it's moving, or where it is. */
    readonly camera: Point;
    readonly config: LayoutConfig;
    /** Values of the layout's options, with defaults filled in. */
    readonly options: Readonly<Record<string, OptionValue>>;

    /** Finds a window by id. */
    window(id: number): Window | null;

    /**
     * Puts a window at `rect` on the canvas and asks it to take that size. Placing a window where
     * it already is does nothing, so an engine can place every window after each change.
     */
    place(win: Window, rect: Rect, opts?: { animate?: boolean }): void;
    /** Shows or hides a window, for example the inactive tabs of a tabbed column. */
    setVisible(win: Window, visible: boolean): void;
    /** Tells a window to be fullscreen, maximized or normal. The layout still places it. */
    setMode(win: Window, mode: SizingMode): void;
    /** Moves a window to the top, so it draws above windows that it overlaps. */
    raise(win: Window): void;
    /** Gives a window focus, or none. */
    focus(win: Window | null): void;
    /**
     * Moves the camera. Coordinates left out stay as they are. While a swipe gesture moves the
     * camera, bento ignores this; the layout gets `gestureEnd` when the swipe ends.
     */
    moveCamera(to: Partial<Point>, opts?: { animate?: boolean }): void;
    /** Stores anything JSON-serializable on a window, see `Window.data`. */
    setWindowData(win: Window, data: unknown): void;
    /**
     * Shows a highlight on the canvas, like around a window picked for a two-step action, or
     * hides it with `null`. While a window is dragged, bento highlights `dropTarget` instead.
     * Setting the same highlight again does nothing.
     */
    setHint(rect: Rect | null): void;
    /**
     * Draws a tab indicator for windows shown as tabs, or removes it with `null`. `key` tells
     * tab bars apart, so that each keeps its animations; any string. Clicking a tab activates its
     * window through `Layout.activate`. Hiding inactive tabs is up to the engine. Setting the same
     * tab bar again does nothing.
     */
    setTabBar(key: string, bar: TabBar | null): void;
    /** Keys of the tab bars shown, to remove the ones that went away. */
    readonly tabBars: readonly string[];
    /**
     * Space the tab indicator takes inside a tabbed area of `count` tabs, from the config: `w` and
     * `h` to leave free across and down, and `x` and `y` for how far the tabs start from the
     * area's left and top edges. All 0 when the indicator goes outside the area or is hidden.
     */
    tabIndicatorSpace(count: number): Rect;
    /** Rounds a coordinate to physical pixels. */
    round(value: number): number;
    /** Writes to bento's log. */
    log(...args: unknown[]): void;
}

/**
 * A layout for one workspace, as returned by the module's default export.
 *
 * Every window in the layout must have a rect after each call; a new window that the layout
 * doesn't place gets put in the middle of the view.
 */
export interface Layout {
    /** A window is added. Place it, and focus it if `how.activate` is set. */
    windowAdded(win: Window, how: AddInfo): void;
    /** A window is removed. It's already gone from `ws.windows`. */
    windowRemoved(win: Window, how: { reason: RemoveReason }): void;

    /**
     * The layout takes over a workspace that already has windows: at startup of the layout, when
     * the config switches layouts, or when the layout file is reloaded. The windows' `rect` is
     * where they were. If the same layout saved state with `save`, it gets it back as `state`.
     *
     * Defaults to adding each window with reason `"adopt"`.
     */
    adopt?(windows: Window[], focused: Window | null, state: unknown): void;
    /** State to keep when the layout is reloaded. Anything JSON-serializable. */
    save?(): unknown;

    /** Runs an action. Returns whether it did anything, for actions that fall back to others. */
    action?(name: string, args: unknown[], target: Window | null): boolean;
    /** Something wants focus on a window: a click, an app, or IPC. Defaults to focusing it. */
    activate?(win: Window): void;
    /**
     * The window, or an action, asks for a sizing mode. `request` says exactly what changes:
     * `{ fullscreen: false }` leaves fullscreen, which can go back to maximized, while
     * `{ maximized: false }` leaves maximized. `mode` is the mode that results if nothing else is
     * on. Defaults to `ws.setMode(win, mode)`.
     */
    modeRequested?(
        win: Window,
        mode: SizingMode,
        request: { fullscreen?: boolean; maximized?: boolean },
    ): void;
    /** A swipe gesture starts moving the camera. Camera moves are ignored until `gestureEnd`. */
    gestureBegin?(): void;

    /** A window's size, limits, title or other properties changed. */
    windowChanged?(win: Window): void;
    /** The output size, working area or scale changed. */
    viewChanged?(): void;
    /** The config changed. */
    configChanged?(): void;

    /** A pointer drag starts resizing a window. Return false to refuse. */
    resizeBegin?(win: Window, resize: Resize): boolean;
    resizeUpdate?(win: Window, resize: Resize): void;
    resizeEnd?(win: Window): void;

    /**
     * A swipe gesture that moved the camera ended. The camera is at `ws.camera`, moving at
     * `velocity` (px/s), and would coast to `projected`. Snap it somewhere with `ws.moveCamera`;
     * the animation continues from the swipe's speed. If the layout doesn't move it, the camera
     * stays where the swipe left it.
     */
    gestureEnd?(velocity: Point, projected: Point): void;

    // Queries. These must not change anything; bento discards changes they make.

    /** Where a window dragged with the pointer at `pointer` would land, to highlight it. */
    dropTarget?(win: Window, pointer: Point, rect: Rect): Rect | null;
    /**
     * Tile size to suggest to a new window before it opens, borders included like every size in
     * this API; 0 lets the window choose.
     */
    newWindowSize?(info: { rules: WindowRules; minSize: Size; maxSize: Size; borders: Size }): Size;
    /**
     * Windows that move along with `win` to another workspace or output, for the actions that move
     * a whole column, like `move-column-to-workspace-down`. Defaults to just `win`. Actions that
     * move one window, like `move-window-to-workspace-down`, only move `win`.
     */
    transferOut?(win: Window): Transfer;
    /** Windows arrive from another workspace. Defaults to adding each with reason `"move"`. */
    transferIn?(windows: Window[], data: unknown, how: { activate: boolean }): void;
    /** Column and row of a window for IPC clients, 1-based, or `null`. */
    ipcPosition?(win: Window): [number, number] | null;
}

export type CreateLayout = (ws: Workspace) => Layout;
