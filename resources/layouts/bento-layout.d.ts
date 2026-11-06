// Types for bento layout engines, API version 2.
//
// A layout engine runs the presentation of the whole session: where windows go, focus, cameras,
// what actions do, and anything it draws. bento runs one instance of it, which sees every monitor,
// workspace and window. It's a single ES module that bento loads from the main config:
//
//     tiling {
//         layout "bento"              // built in; also "niri", or a path to a .js file
//     }
//
// The engine's options, binds and window rules live in its own .kdl file, see
// `docs/engine-design.md`.
//
// The module exports `meta`, describing the engine, and a default function that creates it:
//
//     export const meta = { api: 2, name: "my-engine", actions: { ... }, options: { ... } };
//     export default function create(bento) {
//         bento.addEventListener("windowadd", (e) => { ... });
//     }
//
// bento defines the objects (monitors, workspaces, windows, groups) and owns the workspace
// structure, floating, panels, the lock screen and drawing. Engines read objects through
// read-only properties and change them with methods. A method call can be refused, and returns
// whether it worked. Engines learn about changes from events, the way the DOM works.
//
// Coordinates are logical pixels on a workspace's canvas, which is unbounded in every direction.
// A workspace's camera is the top-left corner of the visible view on its canvas. Window rects
// include borders, see `Window.borders`.
//
// Engines run in a sandbox: there is no file, network or module access, and every call has a
// time and memory limit. Calls are synchronous, and bento applies what a call changed when it
// returns. API version 1 engines (`bento-layout-v1.d.ts`) still run, one instance per workspace.

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

/** A size from options or window rules, written `"50%"` or `"800"` in the .kdl file. */
export type Preset = { proportion: number } | { fixed: number };

/**
 * A size change from an action argument of type `"size"`, written like `"+10%"`. `"%"` values are
 * percentages of the working area.
 */
export interface SizeChange {
    op: "set" | "adjust";
    value: number;
    unit: "px" | "%";
}

/** How a window is sized, as the window is told. */
export type SizingMode = "normal" | "fullscreen" | "maximized";

// Declarations.

export type ValueType = "boolean" | "number" | "string" | "choice" | "size" | "color" | "gradient";

/**
 * An option, or a property an engine's window rules can set. Values are written in the engine's
 * .kdl file: `center-focused-column "never"`, `preset-column-widths "33%" "50%" "67%"`.
 */
export interface OptionSpec {
    /**
     * `"size"` values reach the engine as a `Preset`. A size written with no value, like
     * `default-column-width` alone on its line, reaches it as `null`: the window decides.
     * `"color"` values are written `"#rrggbb"` or `"#rrggbbaa"` and reach the engine as a
     * `Color`. `"gradient"` values are written with properties, like
     * `active-gradient from="#80c8ff" to="#c7ff7f" angle=45 relative-to="view"`, and reach the
     * engine as a `Gradient`, or `null` when left out.
     */
    type: ValueType;
    /**
     * Whether the value is a list, written as several arguments. A gradient is written with the
     * properties of its node, so it can't be a list.
     */
    list?: boolean;
    /**
     * Written the way the .kdl file writes it: `"50%"`, or `["33%", "50%"]` for a list; `null` for
     * a size left to the window, or for no gradient. A window rule property that no rule sets is
     * left out of `Window.rules`, so its default is only for settings screens to show.
     */
    default: unknown;
    /** The values a `"choice"` accepts. */
    choices?: string[];
    description?: string;
}

/**
 * An argument of an action. bento checks binds against it, and settings screens use it to offer
 * the right field.
 */
export interface ArgSpec {
    name: string;
    /**
     * Binds write an action's arguments as plain values, so there are no `"color"` or
     * `"gradient"` arguments.
     *
     * - `"size"`: written like `"50%"`, `"+10%"`, `"800"` or `"-100"`; the engine gets a
     *   `SizeChange`.
     * - `"workspace"`: written as an index on the focused monitor, counting from 1, or a
     *   workspace name; the engine gets a `Workspace`, or `null` when there's no such workspace.
     * - `"monitor"`: written as a connector name like `"DP-2"`; the engine gets a `Monitor`, or
     *   `null`.
     */
    type: Exclude<ValueType, "color" | "gradient"> | "workspace" | "monitor";
    /** Value when the argument is left out. Arguments without one must be given. */
    default?: boolean | number | string;
    choices?: string[];
    description?: string;
}

export interface ActionSpec {
    description?: string;
    /** The action's arguments, in order. An action without `args` takes none. */
    args?: ArgSpec[];
}

export interface Meta {
    api: 2;
    name: string;
    description?: string;
    /**
     * Every action the engine has. bento only sends an engine the actions it declares. An action
     * can't take the name of one of bento's own actions: the engine fails to load if it does.
     */
    actions?: Record<string, ActionSpec>;
    options?: Record<string, OptionSpec>;
    /**
     * Properties the engine's window rules can set, like `default-column-width`. Rules are
     * written in the engine's .kdl file with the same `match` and `exclude` as bento's own, and
     * reach the engine resolved, as `Window.rules`.
     */
    windowRules?: Record<string, OptionSpec>;
    /**
     * Whether bento's floating layer exists under this engine; true by default. With it off,
     * windows that ask to float stay with the engine, which gets `floatrequest`.
     */
    floating?: boolean;
}

// Events.

/** A DOM-style event. Listeners run in the order they were added. */
export interface BentoEvent {
    readonly type: string;
    /** The object the event is for. */
    readonly target: unknown;
    readonly defaultPrevented: boolean;
    /** Stops bento's default for a cancelable event, where its description says what that is. */
    preventDefault(): void;
    /** Stops a node event from bubbling to the node's parents. */
    stopPropagation(): void;
}

/** An event that asks the engine for an answer. bento uses the first answer given. */
export interface QueryEvent<T> extends BentoEvent {
    respondWith(value: T): void;
}

export interface EventTarget<Events> {
    addEventListener<K extends keyof Events>(type: K, listener: (e: Events[K]) => void): void;
    removeEventListener<K extends keyof Events>(type: K, listener: (e: Events[K]) => void): void;
}

/** Why a window comes to the engine. */
export type AddReason =
    /** A new window opened. */
    | "open"
    /** The window was there when the engine started, see `Bento.savedState`. */
    | "adopt"
    /** The window was dropped at the end of a pointer drag. */
    | "drop"
    /** The window stopped floating. */
    | "tile";

export interface WindowAddEvent extends BentoEvent {
    readonly window: Window;
    readonly reason: AddReason;
    /** Whether the window should get focus. */
    readonly activate: boolean;
    /**
     * Where the window was on its workspace's canvas: before the engine started, while floating,
     * or where it was dropped. A hint, which the engine is free to ignore.
     */
    readonly rect: Rect | null;
    /** Pointer position on the canvas, when the window was dropped. */
    readonly pointer: Point | null;
    /** For a new window, the window it's related to, like a dialog's parent. */
    readonly nextTo: Window | null;
}

/** Why a window leaves the engine. */
export type RemoveReason = "close" | "drag" | "float";

export interface WindowRemoveEvent extends BentoEvent {
    readonly reason: RemoveReason;
}

/** bento moved a window to another workspace, with one of its own actions. */
export interface WindowMoveEvent extends BentoEvent {
    readonly from: Workspace;
    readonly to: Workspace;
    /** Whether the window should get focus. */
    readonly activate: boolean;
}

/**
 * The window, or an action, asks for a sizing mode. `request` says exactly what changes:
 * `{ fullscreen: false }` leaves fullscreen, which can go back to maximized. `mode` is the mode
 * that results if nothing else is on. Cancelable: by default the window takes `mode`.
 */
export interface ModeRequestEvent extends BentoEvent {
    readonly mode: SizingMode;
    readonly request: { fullscreen?: boolean; maximized?: boolean };
}

/** Something wants focus on a window: a click, an app, or the socket. */
export interface ActivateRequestEvent extends BentoEvent {
    // Cancelable: by default bento shows the window's workspace, focuses its monitor and focuses
    // the window.
}

/** With floating off, a window asks to float. */
export interface FloatRequestEvent extends BentoEvent {
    readonly reason: "dialog" | "rule" | "action";
    /** For a dialog, its parent. */
    readonly parent: Window | null;
    /** The size the window asked for, or null. */
    readonly size: Size | null;
}

export interface ActionEvent extends BentoEvent {
    readonly name: string;
    /** The arguments, with defaults filled in and values converted by type, see `ArgSpec`. */
    readonly args: unknown[];
    /** The window the action is for: the focused one, or one given on the command line. */
    readonly window: Window | null;
}

export interface Edges {
    left: boolean;
    right: boolean;
    top: boolean;
    bottom: boolean;
}

/**
 * A pointer drag starts resizing a window. Cancelable: bento refuses the resize when no listener
 * handles it or one prevents it.
 */
export interface ResizeEvent extends BentoEvent {
    readonly edges: Edges;
    /** How far the pointer has moved since the drag started. */
    readonly delta: Point;
}

/** A pointer drag carries a window over a workspace, for the engine to show where it would go. */
export interface DragEvent extends BentoEvent {
    readonly window: Window;
    /** Pointer position on the workspace's canvas. */
    readonly pointer: Point;
    /** Where the dragged window is drawn. */
    readonly rect: Rect;
}

/**
 * A horizontal swipe on a workspace. bento only reports how the fingers move; the engine moves
 * the camera, or does anything else, with it.
 */
export interface SwipeEvent extends BentoEvent {
    /** How far the fingers moved since the last event, in logical pixels. */
    readonly delta: number;
    /** Speed at the end of the swipe, in px/s; 0 before the end. */
    readonly velocity: number;
    readonly touchpad: boolean;
}

export interface NewWindowSizeQuery extends QueryEvent<Size> {
    /** The workspace the window will open on. */
    readonly workspace: Workspace;
    readonly rules: Readonly<Record<string, unknown>>;
    readonly minSize: Size;
    readonly maxSize: Size;
    readonly borders: Size;
}

export interface UnloadEvent extends BentoEvent {
    /**
     * Keeps state for the next instance of this engine, after a reload. Anything
     * JSON-serializable. It comes back as `Bento.savedState`.
     */
    saveState(state: unknown): void;
}

/** Modifiers held. */
export interface Modifiers {
    shift: boolean;
    ctrl: boolean;
    alt: boolean;
    super: boolean;
}

export interface KeyEvent extends BentoEvent {
    /** The key's xkb name with the held modifiers applied, like `"a"`, `"A"`, `"Return"`, `"Left"`. */
    readonly key: string;
    /**
     * What the key types with the keyboard layout, like `"a"` or `" "`, or `""` for keys that
     * type nothing printable, like arrows, Return, and keys with Ctrl held.
     */
    readonly text: string;
    readonly modifiers: Modifiers;
    /** Whether the key is held down and repeating. */
    readonly repeat: boolean;
}

export interface BentoEvents {
    windowadd: WindowAddEvent;
    monitoradd: BentoEvent & { readonly monitor: Monitor };
    workspaceadd: BentoEvent & { readonly workspace: Workspace };
    /**
     * A group came to exist, whoever made it: the engine, a native group action, or a previous
     * instance of the engine. The groups there are when the engine starts are announced after its
     * windows.
     */
    groupadd: BentoEvent & { readonly group: Group };
    action: ActionEvent;
    /** The keyboard focus moved, to a window or to an empty workspace. */
    focuschange: BentoEvent;
    /** bento's config or the engine's options changed. */
    configchange: BentoEvent;
    newwindowsize: NewWindowSizeQuery;
    unload: UnloadEvent;
    /** A key, while the engine holds the keyboard, see `Bento.grabKeyboard`. */
    keydown: KeyEvent;
    keyup: KeyEvent;
    /** The keyboard went back to the apps, see `Bento.grabKeyboard`. */
    keyboardrelease: BentoEvent & {
        readonly reason: "engine" | "escape" | "click" | "lock" | "focus";
    };
    /** The modifiers held changed, whether or not the engine holds the keyboard. */
    modifierschange: BentoEvent & { readonly modifiers: Modifiers };
}

export interface MonitorEvents {
    /** Its size, scale, working area or neighbours changed. */
    change: BentoEvent;
    remove: BentoEvent;
}

export interface WorkspaceEvents {
    /** Its name, monitor, place in the stack, or whether it's shown changed. */
    change: BentoEvent;
    remove: BentoEvent;
    /** bento moved the camera itself, while a dragged window is near the view's edge. */
    camerachange: BentoEvent;
    dragenter: DragEvent;
    dragmove: DragEvent;
    dragleave: BentoEvent;
    swipebegin: SwipeEvent;
    swipeupdate: SwipeEvent;
    swipeend: SwipeEvent;
}

export interface WindowEvents {
    /** Its title, app id, size, size limits, urgency or rules changed. */
    change: BentoEvent;
    remove: WindowRemoveEvent;
    move: WindowMoveEvent;
    moderequest: ModeRequestEvent;
    activaterequest: ActivateRequestEvent;
    floatrequest: FloatRequestEvent;
    resizebegin: ResizeEvent;
    resizeupdate: ResizeEvent;
    resizeend: BentoEvent;
    /** A double click on one of the window's resize edges. */
    edgedoubleclick: BentoEvent & { readonly edges: Edges };
}

export interface GroupEvents {
    /**
     * Windows joined or left it, or its active window changed, whoever did it. When the active
     * window leaves, the window that takes its place in `windows` becomes active, or the last one.
     */
    change: BentoEvent;
    /** The group is gone: it was dissolved, or fewer than two windows are left. */
    remove: BentoEvent;
}

export interface NodeEvents {
    click: PointerEvent;
    pointerdown: PointerEvent;
    pointerup: PointerEvent;
    pointermove: PointerEvent;
    pointerenter: PointerEvent;
    pointerleave: PointerEvent;
    wheel: PointerEvent & { readonly delta: Point };
}

/**
 * A pointer event on a drawn node. It bubbles to the node's parents, except `pointerenter` and
 * `pointerleave`, which each node the pointer enters or leaves gets on its own.
 */
export interface PointerEvent extends BentoEvent {
    /** Pointer position on the workspace's canvas. */
    readonly pointer: Point;
    /** `1` left, `2` middle, `3` right; 0 for moves. */
    readonly button: number;
    readonly modifiers: { shift: boolean; ctrl: boolean; alt: boolean; super: boolean };
}

// Objects.

/** bento's native settings that engines may follow: what works under every engine. */
export interface Config {
    readonly gaps: number;
    /** Space to leave around tiled windows, already taken out of `Workspace.tilingArea`. */
    readonly struts: { readonly left: number; readonly right: number; readonly top: number; readonly bottom: number };
}

/** What `create(bento)` receives. */
export interface Bento extends EventTarget<BentoEvents> {
    readonly monitors: readonly Monitor[];
    /** The monitor with keyboard focus. */
    readonly focusedMonitor: Monitor | null;
    /** The window with keyboard focus, if the engine places it. */
    readonly focused: Window | null;
    /** Every window the engine places, on every workspace. Floating windows aren't the engine's. */
    readonly windows: readonly Window[];
    readonly groups: readonly Group[];
    /**
     * The engine's options, from its .kdl file, with defaults filled in. A workspace's own options,
     * which can differ per monitor and per named workspace, are `Workspace.options`.
     */
    readonly options: Readonly<Record<string, unknown>>;
    /** State the previous instance of this engine saved in `unload`, or `null`. */
    readonly savedState: unknown;

    window(id: number): Window | null;
    workspace(id: number): Workspace | null;
    monitor(name: string): Monitor | null;

    /** Groups windows, which leave any group they're in. Returns null for fewer than two. */
    createGroup(windows: Window[], active?: Window): Group | null;

    /**
     * Makes the engine's state readable from outside: the shell, bento Settings and scripts read
     * it through bento's socket. bento calls `provider` when someone reads it, and the result
     * must be JSON-serializable. Its shape is the engine's own.
     */
    exposeState(provider: () => unknown): void;
    /** Tells those watching the engine's state that it changed. */
    stateChanged(): void;

    /** Creates a node to draw, see `Node`. It shows once it's added to a workspace's tree. */
    createNode(kind: "box" | "text" | "image", props?: NodeProps): Node;

    /**
     * Calls `callback` once, after `ms` milliseconds, between events like an event of its own.
     * Returns an id for `clearTimeout`. Timers end when the engine is unloaded or reloaded. A
     * delay can't be longer than an hour, and at most 128 timers can be waiting: `setTimeout`
     * throws beyond that. Negative delays count as 0.
     */
    setTimeout(callback: () => void, ms: number): number;
    clearTimeout(id: number): void;

    /**
     * Takes the keyboard for a short interaction the person started, like picking a window by a
     * letter shown on it. While the engine holds it, keys go to `keydown`/`keyup` and apps get
     * none. Escape ends it unless a `keydown` listener prevents its default, and so do a click
     * outside what the engine draws, the lock screen, keyboard focus moving to something that
     * isn't the engine's, a search starting (with `"focus"`), and the engine failing;
     * `keyboardrelease` says which. bento's binds that can't be inhibited keep working. It can only
     * be taken while handling something the person did: an `action`, a `keydown`, or a
     * `pointerdown` or `click` on a node. Returns false otherwise, and when the keyboard can't be
     * taken, like on the lock screen or while a search runs.
     */
    grabKeyboard(): boolean;
    /** Gives the keyboard back to the apps. */
    releaseKeyboard(): void;
    /** Whether the engine holds the keyboard. */
    readonly keyboardGrabbed: boolean;
    /** The modifiers held right now. */
    readonly modifiers: Modifiers;

    /**
     * Starts searching windows: a group's, a workspace's, or every window with `null`. bento owns
     * the query, typing and matching; the engine places and draws the field, `Search.field`, and
     * shows the results. Returns null if a search is already running.
     */
    search(scope: Group | Workspace | null): Search | null;
    /** The running search, or null. */
    readonly searching: Search | null;

    log(...args: unknown[]): void;
}

export interface Monitor extends EventTarget<MonitorEvents> {
    /** Connector name, like `"DP-2"`. */
    readonly name: string;
    readonly size: Size;
    readonly scale: number;
    /** The monitor minus panels and other exclusive layer surfaces, in view coordinates. */
    readonly workingArea: Rect;
    /** Its vertical stack of workspaces, top to bottom. */
    readonly workspaces: readonly Workspace[];
    readonly activeWorkspace: Workspace;
    readonly focused: boolean;
    /** Neighbouring monitors, from their positions, or null. */
    readonly left: Monitor | null;
    readonly right: Monitor | null;
    readonly up: Monitor | null;
    readonly down: Monitor | null;

    /** Moves keyboard focus to this monitor: to its active workspace's focused window, if any. */
    focus(): boolean;
    /** Snaps a coordinate to physical pixels on this monitor. */
    round(value: number): number;
}

export interface Workspace extends EventTarget<WorkspaceEvents> {
    readonly id: number;
    readonly name: string | null;
    readonly monitor: Monitor;
    /** Place in its monitor's stack, from 0 at the top. */
    readonly index: number;
    /** Windows the engine places on it, bottom to top. */
    readonly windows: readonly Window[];
    /** The window that has focus when this workspace does, or null. */
    readonly focused: Window | null;
    /** Whether it's the one its monitor shows. */
    readonly active: boolean;
    readonly camera: Camera;
    /** bento's settings for this workspace, which can differ per monitor or workspace. */
    readonly config: Config;
    /**
     * The engine's options for this workspace: `Bento.options`, with what the engine's file sets
     * for this workspace's monitor (`output "DP-2" { options {} }`) and then for this workspace by
     * name (`workspace "web" { options {} }`) on top. `configchange` tells when they change.
     */
    readonly options: Readonly<Record<string, unknown>>;
    /** The monitor's working area minus the config's struts, starting at a physical pixel. */
    readonly tilingArea: Rect;
    /** The root of what's drawn on this workspace: window nodes and the engine's own nodes. */
    readonly tree: Node;

    /** Switches its monitor to it. */
    show(opts?: { animate?: boolean }): boolean;
    /** Moves it to another monitor, or within its stack, at `index` or at the end. */
    moveTo(monitor: Monitor, index?: number): boolean;
    /** Names it, or unnames it with null. */
    rename(name: string | null): boolean;
}

export interface Window extends EventTarget<WindowEvents> {
    readonly id: number;
    readonly appId: string | null;
    readonly title: string | null;
    readonly workspace: Workspace;
    /** Where the engine last placed it, or null if it hasn't yet. */
    readonly rect: Rect | null;
    /** The size it actually took. Windows don't always take the size they're given. */
    readonly size: Size;
    /** Smallest size it accepts; 0 means no limit. */
    readonly minSize: Size;
    /** Largest size it accepts; 0 means no limit. */
    readonly maxSize: Size;
    /** Total width and height of its borders, included in its rect. */
    readonly borders: Size;
    readonly mode: SizingMode;
    readonly visible: boolean;
    readonly urgent: boolean;
    /** The engine's window rules that match it, resolved, by property name. */
    readonly rules: Readonly<Record<string, unknown>>;
    readonly group: Group | null;
    /** The window's node in its workspace's tree. */
    readonly node: Node;
    /** What this engine stored on it, or null. It survives engine reloads. */
    readonly data: unknown;

    /**
     * Puts it at `rect` on its workspace's canvas and asks it to take that size. Placing a window
     * where it already is does nothing. Returns false if the window refuses the size.
     */
    place(rect: Rect, opts?: { animate?: boolean }): boolean;
    /**
     * Makes it its workspace's focused window. It gets keyboard focus when its workspace is the
     * focused monitor's active one; `focus` doesn't switch workspaces or monitors.
     */
    focus(): boolean;
    /** Draws it above its siblings in the tree. */
    raise(): boolean;
    /** Moves it to another workspace, on any monitor. The engine places it there. */
    moveTo(workspace: Workspace): boolean;
    /** Tells it to be fullscreen, maximized or normal. The engine still places it. */
    setMode(mode: SizingMode): boolean;
    /** Shows or hides it, like the inactive windows of a group shown as tabs. */
    setVisible(visible: boolean): boolean;
    setData(data: unknown): void;
    /**
     * Starts moving it with the pointer, like dragging a title bar the engine draws. Only from a
     * `pointerdown` listener, with the button still held; bento runs the move as when it starts
     * one, and the window leaves the engine with reason `"drag"` and comes back with `"drop"`.
     * The press belongs to the move from then on: the node gets no `pointerup` or `click` for it.
     * Returns false outside a press, and when the press began a move or resize already.
     */
    beginMove(): boolean;
    /**
     * Starts resizing it from `edges` with the pointer, like `beginMove`; `resizebegin` follows,
     * and bento refuses the resize like any other if no listener takes it. Returns false for no
     * edge at all.
     */
    beginResize(edges: Edges): boolean;
    /**
     * Turns bento's border, focus ring and shadow around it on or off; all on by default. Without
     * the border the window takes the whole of its rect, and `borders` is 0. They come back on
     * when the window leaves the engine, and when a new instance of the engine starts, since what
     * the engine drew in their place is gone too.
     */
    setDecorations(decorations: { border?: boolean; focusRing?: boolean; shadow?: boolean }): void;
    readonly decorations: { readonly border: boolean; readonly focusRing: boolean; readonly shadow: boolean };
}

export interface Camera {
    /** Where it's going, if it's moving, or where it is: the canvas point at the view's top left. */
    readonly x: number;
    readonly y: number;
    /**
     * How large the canvas is drawn: 1 at its size, 0.5 shows twice as much of it. `x` and `y`
     * stay the canvas point at the view's top left, and windows aren't told a new size. Pointer
     * input lands where things are drawn at any zoom. bento keeps it between 0.01 and 100.
     */
    readonly zoom: number;

    /**
     * Moves it, animating `x`, `y` and `zoom` the same way. Values left out stay as they are. A
     * `zoom` that isn't above 0 throws. `velocity` continues a swipe's speed into the animation,
     * in px/s.
     */
    move(to: Partial<Point & { zoom: number }>, opts?: { animate?: boolean; velocity?: number }): boolean;
}

/**
 * A search of windows, see `Bento.search`. bento owns the query, the typing (with input methods
 * and screen readers) and the matching on titles and app ids.
 */
export interface Search extends EventTarget<SearchEvents> {
    readonly scope: Group | Workspace | null;
    readonly query: string;
    /**
     * The windows in scope that match the query, best match first; all of them for no query.
     * Windows that match equally well are in the engine's order: that of `Group.windows`,
     * `Workspace.windows` or `Bento.windows`.
     */
    readonly matches: readonly Window[];
    /**
     * The field people type in, a node of kind `"search"` that bento draws: the query, its caret
     * and selection, and a placeholder. The engine adds it to a tree and sets its `rect`, `font`,
     * `color`, `background`, `border`, `cornerRadius` and the like. The text is left-aligned,
     * inset from the node's edge by half a line, and scrolls to keep the caret in view; the
     * caret and the selection take `color`, and the placeholder takes it dimmed. bento takes the
     * field out of its tree when the search ends.
     */
    readonly field: Node;
    /** Ends it without picking a window, as a click anywhere but on the field does. */
    cancel(): void;
}

export interface SearchEvents {
    /** The query or the matching windows changed. */
    change: BentoEvent;
    /**
     * It ended. With Enter, `picked` is the best match, and an `activaterequest` for it follows;
     * with Escape or `cancel`, it's null.
     */
    end: BentoEvent & { readonly picked: Window | null };
}

/**
 * Windows grouped together, and which one is active. Which windows are grouped is bento's state;
 * order, looks and layout are the engine's.
 */
export interface Group extends EventTarget<GroupEvents> {
    readonly id: number;
    readonly windows: readonly Window[];
    readonly active: Window;

    add(win: Window): boolean;
    remove(win: Window): boolean;
    activate(win: Window): boolean;
    /** Ungroups its windows. */
    dissolve(): void;
}

// Drawing.

/**
 * How a property animates when it changes, like a CSS transition: the name of one of bento's
 * animations from the config, like `"window-movement"`, or an easing, with its `duration` in
 * milliseconds, or a spring.
 */
export type Transition =
    | string
    | { duration: number; curve: "linear" | "ease-out-quad" | "ease-out-cubic" | "ease-out-expo" }
    | { spring: { dampingRatio: number; stiffness: number; epsilon: number } };

/** A colour, written `"#rgb"`, `"#rrggbb"` or `"#rrggbbaa"`. */
export type Color = string;

/**
 * A linear gradient from `from` to `to`, at `angle` degrees (0 goes up, 90 to the right),
 * interpolated in the colour space `in`, like niri's: `"srgb"`, `"srgb-linear"`, `"oklab"`, or
 * `"oklch"` followed by `"shorter hue"`, `"longer hue"`, `"increasing hue"` or `"decreasing
 * hue"`. It spans the node's rect, or with `relativeTo: "view"` the whole view of the workspace,
 * so that neighbouring nodes share one gradient. Left out, `angle` is 180, from top to bottom,
 * `relativeTo` is `"node"` and `in` is `"srgb"`, like niri's. Angles are whole degrees: bento
 * rounds them.
 */
export interface Gradient {
    from: Color;
    to: Color;
    angle?: number;
    relativeTo?: "node" | "view";
    in?: string;
}

export interface NodeProps {
    /**
     * Where it's drawn, on the workspace's canvas. For a window's node this is its place, set by
     * `Window.place`.
     */
    rect?: Rect;
    /**
     * From 0 to 1. It applies to the node and its children together, as one picture, so that
     * overlapping children don't show through each other. A window is drawn on its own, so it gets
     * the opacity of the nodes above it but isn't part of their picture.
     */
    opacity?: number;
    /** Scales and moves it, and its children, around the centre of its rect. */
    transform?: { scale?: number; x?: number; y?: number };
    /**
     * Whether its children, of every kind, are clipped to its rect, with its corner radius. Clips
     * of nested nodes cut one inside the other.
     */
    clip?: boolean;
    cornerRadius?: number;
    /** Fill, for every node but a window's. */
    background?: Color | Gradient | null;
    /** A border drawn inside its rect, for every node but a window's. */
    border?: { width: number; color: Color | Gradient } | null;
    /** `"text"`: what it says, centred in its rect and cut with an ellipsis where it's too wide. */
    text?: string;
    /** `"text"`: a Pango font description, like `"sans 11"`, with its size in logical pixels. */
    font?: string;
    /** `"text"`: its colour. */
    color?: Color;
    /**
     * `"image"`: an icon name from the icon theme, or an absolute path, drawn as large as fits
     * its rect, centred. The icon theme is the one apps on the session use, from GNOME's
     * interface settings or `gtk-icon-theme-name` in `~/.config/gtk-4.0/settings.ini`, with the
     * themes it inherits from and `hicolor`. Icons are PNGs or SVGs, and a path can be a PNG,
     * JPEG, WebP or SVG. An SVG is drawn at the size of the node, and shows no text or images
     * that it embeds.
     */
    image?: string;
    /** Whether it gets pointer events. Nodes without listeners can leave it off. */
    interactive?: boolean;
    /**
     * A window's node: where pointer input on it goes. `"app"`, the default, gives it to the app
     * where the window is drawn, through its transform and the camera's zoom; `"engine"` gives it
     * to the engine as the node's pointer events, the app getting none; `"none"` lets it through
     * to what's below.
     */
    input?: "app" | "engine" | "none";
    /** `"search"`: what the field shows while the query is empty. */
    placeholder?: string;
    /**
     * How each property animates when it changes; properties left out change at once. A window's
     * node animates the window's movement with its `rect` transition, instead of bento's
     * `"window-movement"`; its size still animates as bento's `"window-resize"` says.
     */
    transitions?: Partial<Record<Transitioned, Transition>>;
    /**
     * The values of some properties that the node starts from when it enters a tree, which it
     * then goes from to its own values with its `transitions`, like CSS's `@starting-style`: a
     * tab indicator that grows in has an `enter` rect that is zero wide. Properties without a
     * transition don't start anywhere else. It's for before the node is added to a tree, and
     * `set` throws if it changes it for a node that is in one. A node that's taken out of a tree
     * and added to one again enters again.
     */
    enter?: Partial<Pick<NodeProps, Transitioned>>;
}

/** The properties that can have a transition, and an `enter` value. */
export type Transitioned = "rect" | "opacity" | "transform" | "cornerRadius" | "background";

/**
 * Something drawn on a workspace: a box, text, an image, or a window. Nodes form a tree per
 * workspace, rooted at `Workspace.tree`; children draw above their parent, later siblings above
 * earlier ones. bento draws the tree and runs its transitions, so no engine code runs per frame.
 */
export interface Node extends EventTarget<NodeEvents> {
    readonly kind: "root" | "box" | "text" | "image" | "window" | "search";
    /** The window, for a window's node. */
    readonly window: Window | null;
    readonly parent: Node | null;
    readonly children: readonly Node[];
    readonly props: Readonly<NodeProps>;

    /** Changes some properties. Those with a transition animate to their new value. */
    set(props: NodeProps): void;
    /** Adds a child last, or before `before`, moving it from wherever it was. */
    append(child: Node, before?: Node): void;
    /** Takes it out of the tree. A window's node can't be removed, only moved. */
    remove(): void;
}

/** The module's default export. */
export type CreateEngine = (bento: Bento) => void;
