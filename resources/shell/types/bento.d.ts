// bento's shell API: what a shell, or one of its modules, is written against.
//
// A shell is TSX with Solid 2.0: components that run once, and values that update exactly what
// reads them. bento compiles `.tsx`, `.ts` and `.jsx` files itself as it loads them, with Solid's
// own compiler, so there's nothing to install or build. A file can hold a whole shell or one
// module, and imports others:
//
//   import { Surface, now } from "bento";
//   import { createSignal, For, Show } from "solid-js";
//   import { Clock } from "bento:modules/clock";
//   import { Bar } from "./bar";
//
// The shell that runs is chosen in bento's config, `shell { script "windows" }`, or in bento's
// settings: one of bento's own shells by name, or a file of one's own, by its path, which bento
// loads again when any of its files changes. Its default export is the root component. One of
// bento's own is a start for one's own:
//
//   export { default } from "bento:shells/unix";
//
// `bento:modules/<name>` is bento's module of that name, unless `~/.config/bento/shell/modules/
// <name>.tsx` is there, which is used in its place everywhere, by bento's own modules too.
// `bento:stock/modules/<name>` is always bento's own, for a module of one's own to build on. An
// override exports the same names, with the same props. `bento:shells/<name>` and
// `bento:stock/shells/<name>` are the same for whole shells.
//
// bento's own shells are `unix`, islands along the top, and `windows`, a taskbar along the
// bottom. What modules there are, and what each has, an editor shows: bento keeps a copy of its
// modules, its shells, this file and Solid's types in `~/.local/share/bento/shell/`, and a
// `tsconfig.json` in `~/.config/bento/shell/` that points an editor there.
//
// This is the first cut of the API. The scene doesn't have paths, masks, gradients, shadows,
// blend modes or 3D transforms yet.

/** Calls `fn` once what's running now is done. */
declare function queueMicrotask(fn: () => void): void;

interface ImportMeta {
  /** The module's name: its file's name without the extension, like `taskbar`, which its
   * settings are kept by. */
  readonly name: string;
}

declare module "bento" {
  import type { Accessor, Element as SolidElement } from "solid-js";

  // ---------------------------------------------------------------------------------------------
  // Styles
  //
  // Styles are CSS's, written as objects: flexbox, margins, start and end, states, and text
  // styles that what a node holds inherits, with bento's own properties for what CSS has no word
  // for, like materials. A node's `style` is one object, or a list of them with later ones
  // winning, where `false`, `null` and `undefined` are skipped: `[s.button, on() && s.on]`.
  //
  // A node's place and size, corners, opacity, scale, blur, translation and drawn size move
  // there with bento's springs whenever they change, unless it says `animate: false`; they keep
  // their speed when sent somewhere else on the way, and never overshoot. Colours, borders and
  // text styles change at once.
  // ---------------------------------------------------------------------------------------------

  /** A colour: one of the theme's, at an opacity, or a fixed one. The theme's follow libadwaita,
   * light or dark: `fg` is the text colour, `accent` the system's accent, `window` the window
   * background. `"accent/0.6"` is the accent at 60%. */
  export type ThemeColor = "fg" | "accent" | "window";
  export type Paint = ThemeColor | `${ThemeColor}/${number}` | `#${string}`;

  /** Logical pixels, or a percentage of the node's own size. */
  export type Offset = number | `${number}%`;

  /** Logical pixels, a percentage of the parent, or `"auto"`. */
  export type Length = number | `${number}%` | "auto";

  /** All four sides, vertical and horizontal, or top, right, bottom and left, as in CSS. */
  export type Sides<T = number> = T | [T, T] | [T, T, T] | [T, T, T, T];

  /** What a shape is made of. `glass`, `acrylic` and `mica` are bento's materials, set in its
   * settings, the same behind the shell as behind apps:
   * - `solid`: its `background` alone;
   * - `mica`: the wallpaper alone, blurred and tinted, never the windows behind;
   * - `acrylic`: what's behind, blurred and tinted;
   * - `glass`: what's behind, blurred and refracted through a bezel.
   * Shapes of glass, acrylic or mica that come close melt together, as one piece, when they're
   * in surfaces of the same layer and the same side of `z`. */
  export type Material = "none" | "solid" | "mica" | "acrylic" | "glass";

  export type Align = "start" | "center" | "end" | "stretch" | "baseline";

  /** What's moved by springs, for `transition`. `frame` is a node's place and size from the
   * layout. */
  export type Motion = "frame" | "opacity" | "radius" | "scale" | "blur" | "translate" | "drawSize";

  export interface LayoutStyle {
    /** `"none"` leaves it out of the layout: neither drawn nor hit, nor taking room. Hidden and
     * shown with it, a node goes to its `exit` pose first, and comes from its `enter` pose. */
    display?: "flex" | "none";
    /** `"hidden"` leaves it undrawn and deaf to the pointer, keeping its room. */
    visibility?: "visible" | "hidden";
    flexDirection?: "row" | "column" | "row-reverse" | "column-reverse";
    flexWrap?: "nowrap" | "wrap";
    justifyContent?: "start" | "center" | "end" | "space-between" | "space-around" | "space-evenly";
    alignItems?: Align;
    alignSelf?: Align;
    alignContent?: "start" | "center" | "end" | "stretch" | "space-between" | "space-around";
    flexGrow?: number;
    flexShrink?: number;
    flexBasis?: Length;
    gap?: number;
    rowGap?: number;
    columnGap?: number;
    width?: Length;
    height?: Length;
    minWidth?: Length;
    minHeight?: Length;
    maxWidth?: Length;
    maxHeight?: Length;
    aspectRatio?: number;
    padding?: Sides;
    paddingTop?: number;
    paddingRight?: number;
    paddingBottom?: number;
    paddingLeft?: number;
    margin?: Sides<number | "auto">;
    marginTop?: number | "auto";
    marginRight?: number | "auto";
    marginBottom?: number | "auto";
    marginLeft?: number | "auto";
    /** `"absolute"` takes it out of the flow, placed by `top`, `right`, `bottom` and `left` in
     * its parent. */
    position?: "relative" | "absolute";
    top?: Length;
    right?: Length;
    bottom?: Length;
    left?: Length;
    inset?: Sides<Length>;
  }

  export interface LookStyle {
    background?: Paint;
    material?: Material;
    /** A material's tint, over bento's own shade. */
    tint?: Paint;
    /** Each corner's radius: all, or top left, top right, bottom right and bottom left. */
    borderRadius?: number | [number, number, number, number];
    borderWidth?: number;
    borderColor?: Paint;
    opacity?: number;
    /** How big it's drawn around its middle, with what it holds, as a fraction of its size. */
    scale?: number;
    /** How far it's drawn from its place, `[x, y]`, without moving anything else: pixels, or a
     * percentage of its own size, like `[0, "100%"]` for its whole height down. */
    translate?: [Offset, Offset];
    /** How big its own shape is drawn in its place, `[width, height]`, without scaling what it
     * holds, in pixels or percentages of its place's size: a shape that grows out of a point is
     * drawn from `[0, 0]`, and one that slides out of its bottom edge from `["100%", 0]`. `null`
     * is its place's size. */
    drawSize?: [Offset, Offset] | null;
    /** Where in its place its shape is held while it's drawn at another size: its middle, an
     * edge, or a corner, like `"bottom right"`, as a panel slides out of a bar from there. */
    drawFrom?: "center" | Edge | `${"top" | "bottom"} ${"left" | "right"}`;
    /** How blurred it's drawn, with what it holds, in logical pixels. */
    blur?: number;
    /** `"hidden"` cuts what it holds to its shape, rounded corners and all; `"scroll"` too, and
     * what it holds scrolls in it with a wheel or a touchpad, as far as it reaches. */
    overflow?: "visible" | "hidden" | "scroll";
    /** How the pointer looks over it: a hand over what has a click handler, unless it says. */
    cursor?: "default" | "pointer" | "text" | "grab";
  }

  /** Text styles. Like CSS, they're inherited: text takes them from the nearest node above it
   * that sets them, and a symbolic icon is drawn in the `color` it inherits. */
  export interface TextStyle {
    color?: Paint;
    fontSize?: number;
    fontWeight?: number | "normal" | "bold";
    fontFamily?: string;
    textAlign?: "start" | "center" | "end";
    fontVariantNumeric?: "normal" | "tabular-nums";
    /** `1` keeps text on one line, cut with an ellipsis where it doesn't fit. */
    lineClamp?: 1 | "none";
  }

  export interface MotionStyle {
    /** How long its moves take, in seconds: all of them, or each kind's. Unset, they take as long
     * as bento's windows do, with bento's animation settings. */
    transition?: number | Partial<Record<Motion, number | null>>;
    /** `false` puts it at its place at once whenever the layout moves it. */
    animate?: boolean;
  }

  export interface Style extends LayoutStyle, LookStyle, TextStyle, MotionStyle {
    /** While the pointer is on it or on what it holds, as in CSS. A node with a `:hover` style
     * takes the pointer. */
    ":hover"?: Style;
    /** While it's pressed. */
    ":active"?: Style;
    /** While it, an input, has the keys. */
    ":focus"?: Style;
  }

  export type StyleProp = Style | false | null | undefined | readonly StyleProp[];

  /** A pose: what a node can start from or end at by springs alone, for `enter`, `exit` and
   * `jump()`, with how fast it goes there. */
  export type Pose = Pick<
    Style,
    "opacity" | "scale" | "blur" | "translate" | "drawSize" | "borderRadius" | "transition"
  >;

  /** Names styles, and checks them: `const s = css({ bar: { ... }, item: { ... } })`. */
  export function css<const T extends Record<string, Style>>(styles: T): T;

  // ---------------------------------------------------------------------------------------------
  // Nodes
  // ---------------------------------------------------------------------------------------------

  /** A rectangle on the output, in logical pixels. */
  export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
  }

  /** What a press, a click or a drag says. `x` and `y` are on the output, `offsetX` and
   * `offsetY` on the node, as it's drawn now, `frame`. */
  export interface PointerEvent {
    /** Where on the output, which a click and a press say. */
    x: number;
    y: number;
    /** The output it's on, which a click and a press say. */
    output?: OutputName;
    offsetX: number;
    offsetY: number;
    frame: Rect;
    /** 1, 2 or 3: left, middle or right. A touch or a pen is 1. A drag doesn't say. */
    button?: 1 | 2 | 3;
  }

  export interface KeyEvent {
    /** Its keysym's name, like "a", "Return", "Escape" or "Down". */
    key: string;
    /** What it types, if anything. */
    text: string;
    pressed: boolean;
    repeat: boolean;
    shift: boolean;
    ctrl: boolean;
    alt: boolean;
    super: boolean;
  }

  /** A node in bento's scene, as a `ref` gets it. */
  export interface SceneNode {
    /** Where its layout puts it, whether it shows or not. As Solid applies changes once a
     * handler is done, `onLayout` is the way to follow it. */
    layout(): Rect | null;
    /** Where it's drawn now, on its way there, with its scale. */
    frame(): Rect | null;
    /** Puts it at `pose` at once, to move from there to where its style says. */
    jump(pose: Pose): void;
    /** Scrolls what it holds to `top` pixels from its top, as far as it goes, for one with
     * `overflow: "scroll"`. */
    scrollTo(top: number): void;
  }

  export interface InputNode extends SceneNode {
    /** Gives it the keys, while its surface has the keyboard. */
    focus(): void;
    blur(): void;
    /** What it holds. */
    readonly text: string;
    /** Replaces what it holds, without telling `onInput`. */
    setText(text: string): void;
  }

  export interface NodeProps {
    style?: StyleProp;
    /** Where it comes in from when it's added or shown, as CSS's `@starting-style`:
     * `enter={{ opacity: 0, scale: 0.9 }}`. */
    enter?: Pose;
    /** Where it goes before it's taken away or hidden: taken away, it stays in the scene, out of
     * the layout's flow at its last place and deaf to the pointer, until it gets there, and then
     * goes. What it holds goes to its own `exit` poses with it. Hidden with `display: "none"`
     * and shown again on the way, it turns around from where it is. */
    exit?: Pose;
    /** Its motion came to rest. */
    onRest?: () => void;
    /** Where its layout puts it changed, from the next frame: to size or place things by it. */
    onLayout?: (rect: Rect) => void;
    /** For one with `overflow: "scroll"`: how far what it holds is scrolled from its top, how
     * far it can be, and how tall it is, when any of them changed, as for a scrollbar. */
    onScrolled?: (event: { top: number; reach: number; height: number }) => void;
    /** Pressed and let go on it, with the left button, a touch or a pen. Having a press handler,
     * `onHover` or `onScroll`, it takes the pointer from what's below it; so does any shape,
     * with a `background`, `material` or border. The rest lets it through. */
    onClick?: (event: PointerEvent) => void;
    onMiddleClick?: (event: PointerEvent) => void;
    onRightClick?: (event: PointerEvent) => void;
    /** Pressed on it, with any button. */
    onPress?: (event: PointerEvent) => void;
    /** A press on it let go, wherever that is. */
    onRelease?: (event: { button: 1 | 2 | 3 }) => void;
    /** The pointer moved while a press on it is held. */
    onDrag?: (event: PointerEvent) => void;
    /** The pointer came onto it, or onto something it holds, or left. */
    onHover?: (event: { inside: boolean }) => void;
    onScroll?: (event: { dx: number; dy: number }) => void;
    /** The pointer came into its place or left it, whatever is there, in the shell or under it.
     * It takes nothing from the pointer: a zone, to know it's near. */
    onNear?: (event: { inside: boolean }) => void;
  }

  export interface BoxProps extends NodeProps {
    ref?: SceneNode | ((node: SceneNode) => void);
    /** Nodes. A string or a number directly in a box is a text of its own; for words with values
     * in them, `<text>` makes one run of them. */
    children?: JSX.Element;
  }

  export interface TextProps extends NodeProps {
    ref?: SceneNode | ((node: SceneNode) => void);
    /** Strings and numbers, as one run of text. */
    children?: string | number | false | null | (string | number | false | null | undefined)[];
  }

  export interface ImageProps extends NodeProps {
    ref?: SceneNode | ((node: SceneNode) => void);
    /** An icon by its name: one of bento's own, Phosphor's `ph-…-symbolic` that the shell has,
     * like `ph-cards-three-symbolic`, or one from the icon theme; or an image file by its path,
     * as tray icons and notifications give them. One whose name ends in `-symbolic` is drawn in
     * the text colour it inherits. */
    icon?: string;
    /** An image file by its path: PNG, JPEG, WebP or SVG. */
    src?: string;
    /** Its width and height, unless its style says. 16 when neither does. */
    size?: number;
  }

  export interface CopyProps extends NodeProps {
    ref?: SceneNode | ((node: SceneNode) => void);
    /** The window it shows, by its id: a live copy of it, fitted into the node and centred,
     * with the node's corner radius, without its border, focus ring or shadow, as an engine's
     * copy node shows one. The window stays where it is. bento draws copies over the rest of
     * their surfaces' layer, so nothing in the shell goes over one. Surfaces under the windows
     * don't show copies. */
    window?: number;
  }

  export interface InputProps extends NodeProps {
    ref?: InputNode | ((node: InputNode) => void);
    /** What it holds, set again whenever it changes. */
    value?: string;
    placeholder?: string;
    /** Every change to what it holds. */
    onInput?: (text: string) => void;
    /** Enter. */
    onSubmit?: (text: string) => void;
    /** Keys it doesn't use itself, like Up and Down: `true` says the key was used. */
    onKey?: (event: KeyEvent) => boolean | void;
    /** Gives it the keys when it's made. */
    autofocus?: boolean;
    placeholderStyle?: StyleProp;
    caretStyle?: StyleProp;
    selectionStyle?: StyleProp;
  }

  // ---------------------------------------------------------------------------------------------
  // Surfaces
  //
  // A surface is what goes on an output: a bar, a panel, a launcher, a wallpaper. Everything else
  // is in one. Surfaces are placed the way Wayland's layer-shell places other programs':
  // anchored to edges, with margins, keeping room from the windows, in a layer. A `<Surface>`
  // can be rendered anywhere, inside another's tree too, as a module's flyout is: it goes on its
  // output all the same, and what's in it knows where it was rendered.
  // ---------------------------------------------------------------------------------------------

  /** An output, by its connector's name, like "DP-1". */
  export type OutputName = string;

  export type Edge = "top" | "right" | "bottom" | "left";

  /** Where a surface goes among what else is on the output, as in layer-shell: from the back,
   * `background`, `bottom`, the windows, `top`, fullscreen windows, `overlay`. Those under the
   * windows move with the workspaces. */
  export type Layer = "background" | "bottom" | "top" | "overlay";

  /** Its style's size and margins are the surface's; its margins are from the output's edges.
   * `position` and the insets mean nothing on a surface. */
  export interface SurfaceProps extends BoxProps {
    /** One output, every output (`"all"`, one surface each), or the one with the focus
     * (`"focused"`, made again on the output the focus goes to). What's in it knows which with
     * `useOutput()`. `"all"` unless it says. */
    output?: OutputName | "all" | "focused";
    /** Its layer. `"top"` unless it says, which is under fullscreen windows; a shell's bar that
     * stays over them is in `"overlay"`, with a negative `z` to stay under screenshot tools. */
    layer?: Layer;
    /** Where it goes in its layer: other programs' surfaces there are at 0, so it's below them
     * with a negative `z` and above them otherwise; among the shell's own, higher is in front,
     * and the same `z` goes in the order they came. 1 unless it says. */
    z?: number;
    /** The edges it's held to, at its margins from them. Held to two opposite edges, it
     * stretches between them unless its style gives its size; else it's as big as what it
     * holds. Held to none, it's in the middle of the output. */
    anchor?: Edge[];
    /** Put beside a node rather than held to edges, as a flyout is over its button, or beside a
     * point on the output, as a menu is at the pointer: on `side` of it, `gap` away, lined up with
     * its `align`. Where there's no room on its side it goes on the other, and where there's no
     * room lined up with its start it lines up with its end, and the other way around; it's kept
     * on the output by its margins. */
    attach?: {
      to: SceneNode | { x: number; y: number };
      side?: Edge;
      align?: "start" | "center" | "end";
      gap?: number;
    };
    /** Keeps room for it along the edge it's held to, that windows stay out of: its size there
     * and its margin with `true`, which follows it as it grows, or that many pixels. It needs to
     * be held to one edge, or to one and both of those beside it. */
    reserve?: boolean | number;
    /** Whether it takes the keyboard from the apps: never, after a press on it until a press
     * elsewhere (`"on-demand"`), or as long as it shows (`"exclusive"`). Of those that want it,
     * the frontmost has it. Keys go to the focused input in it first, then to `onKey`, and
     * Escape, unless something used it, to `onEscape`. */
    keyboard?: "none" | "on-demand" | "exclusive";
    /** A key, while it has the keyboard: `true` says it was used. */
    onKey?: (event: KeyEvent) => boolean | void;
    onEscape?: () => void;
    /** A press anywhere off it: on a window, the desktop or another surface. */
    onOutside?: () => void;
  }

  /** A surface, on one output, every output or the focused one. */
  export function Surface(props: SurfaceProps): JSX.Element;

  /** The output the surface it's in is on. */
  export function useOutput(): OutputName;

  // ---------------------------------------------------------------------------------------------
  // bento's state, as signals and stores: each updates what reads it. Lists keep their items as
  // they change, by their `id`, so rows of a `<For>` stay.
  // ---------------------------------------------------------------------------------------------

  /** The outputs, by name. */
  export const outputs: Accessor<OutputName[]>;

  /** How big an output is, in logical pixels. */
  export function outputSize(output: OutputName): { width: number; height: number } | null;

  /** The output with the focused workspace. */
  export const focusedOutput: Accessor<OutputName | null>;

  export interface Workspace {
    id: number;
    /** Its place on its output, from 0. */
    index: number;
    name: string | null;
    /** Whether it's the one its output shows. */
    active: boolean;
    /** Whether it has the focus. */
    focused: boolean;
  }

  /** The workspaces of an output. */
  export function workspaces(output: OutputName): Workspace[];

  export interface Window {
    id: number;
    /** Its app's id, which is a desktop entry's id without `.desktop`, or an app's `wmClass`. */
    appId: string | null;
    title: string | null;
    workspace: number | null;
    output: OutputName | null;
    focused: boolean;
    urgent: boolean;
    floating: boolean;
  }

  /** The windows, for a taskbar. `action("focus-window", [], { window: id })` raises one. */
  export function windows(): Window[];

  /** Whether `output` shows a fullscreen window. */
  export function fullscreen(output: OutputName): boolean;

  export interface Minimap {
    /** The size of the view, the part of the workspace the output shows. */
    view: [number, number];
    /** The active workspace's tiles, where they're headed relative to the view's top left, in
     * logical pixels: a scrolling workspace's reach past the view on either side. */
    tiles: {
      id: number;
      x: number;
      y: number;
      w: number;
      h: number;
      /** Whether it's the workspace's active window. */
      active: boolean;
      floating: boolean;
    }[];
  }

  /** Where the windows of `output`'s active workspace sit, for a minimap. */
  export function minimap(output: OutputName): Minimap | null;

  /** bento's own style, to line up with. */
  export interface BentoStyle {
    /** The gaps between windows, and from the screen's edges. */
    gaps: number;
    /** The windows' corner radius. */
    cornerRadius: number;
    /** Whether things move at all, and how much slower than usual. */
    animations: boolean;
    slowdown: number;
    /** How long bento's springs take to come most of the way, in seconds. */
    response: number;
    /** Whether bento draws its materials dark, with the system style. */
    dark: boolean;
  }
  export const bentoStyle: Accessor<BentoStyle>;

  /** What bento's config has for the shell: where the user is, which bento's settings set. */
  export interface ShellConfig {
    location: { name: string | null; latitude: number; longitude: number } | null;
  }
  export const config: Accessor<ShellConfig>;

  /** How long `ms` of motion takes with bento's animation settings: slower with a slowdown, and
   * none with animations off. */
  export function paced(ms: number): number;
  /** How long bento's springs take to come most of the way, in milliseconds. */
  export function settling(): number;

  // ---------------------------------------------------------------------------------------------
  // Doing things
  // ---------------------------------------------------------------------------------------------

  /** Runs a bento action, as a bind would: `action("spawn", ["foot"])`,
   * `action("focus-workspace", [], { workspace: id })`. */
  export function action(
    name: string,
    args?: (string | number | boolean)[],
    options?: { properties?: Record<string, unknown>; window?: number; workspace?: number },
  ): void;

  /** Listens to an action a module declares in its `meta`, which a bind runs as
   * `shell-action "name" args…`, with the focused output. Listening inside a component, it stops
   * when the component goes. */
  export function onAction(
    name: string,
    fn: (event: { output: OutputName | null; args: string[] }) => void,
  ): void;

  export interface App {
    /** Its desktop entry's id, like `org.gnome.Nautilus.desktop`. */
    id: string;
    name: string;
    generic?: string;
    comment?: string;
    keywords: string[];
    icon?: string;
    /** The app id its windows have, when it isn't the entry's id without `.desktop`. */
    wmClass?: string;
  }
  /** The apps that show in launchers, sorted by name. */
  export function apps(): App[];
  /** Starts an app by its desktop entry's id. */
  export function launch(id: string): void;
  /** Loads an icon ahead of its first use, at `size`, so it shows in the first frame it's in. */
  export function preload(icon: string, size?: number): void;

  /** Runs `command` with `sh -c`, and says its exit code once it's over. */
  export function run(command: string): Promise<number>;

  /** Shows a notification. A `transient` one is only a toast, and isn't kept. */
  export function notify(
    summary: string,
    body?: string,
    icon?: string,
    options?: { transient?: boolean },
  ): void;

  /** Calls `fn` after `ms`, and gives what cancels it. Inside a component, it's cancelled when
   * the component goes. */
  export function after(ms: number, fn: () => void): () => void;
  /** Calls `fn` every `ms`, and gives what stops it, like `after`. */
  export function every(ms: number, fn: () => void): () => void;
  /** The time, on the mark of every `ms`, 1000 unless it says: `const time = now()`. */
  export function now(ms?: number): Accessor<Date>;

  /** What the shell notes for itself as it's used, its state, kept between runs by key, as a
   * signal and its setter: `const [quiet, setQuiet] = stored("doNotDisturb", false)`. Keys are
   * shared by every module, so a module of one's own puts its name in front of its keys. What
   * the user chooses is a setting instead, see `useModule`. */
  export function stored<T>(key: string, fallback: T): [Accessor<T>, (value: T) => void];

  // ---------------------------------------------------------------------------------------------
  // Settings and actions
  //
  // A module, or a shell, declares its settings and actions in its `meta`, the same way engines
  // do, and `useModule` hands them to bento. A setting is a typed value that a module declares
  // for what it shows, which every shell with the module shares, or a shell declares for itself.
  // The user's are in `~/.config/bento/shell/settings.kdl`, a block for each module and shell by
  // its name, and bento's settings show them:
  //
  //   taskbar {
  //       pinned "firefox" "org.gnome.Nautilus"
  //   }
  //
  // An action is something a bind runs, by its name in its block of the shell's file, and which
  // bento's settings list with bento's own:
  //
  //   export const meta = {
  //     settings: {
  //       pinned: { type: "string", list: true, default: [], description: "Apps pinned to it." },
  //     },
  //     actions: {
  //       "switch-audio-output": { description: "Switch to the next pinned output." },
  //     },
  //   } satisfies Meta;
  //   const { settings, setSetting } = useModule(import.meta, meta);
  //   settings.pinned; // string[]
  //   onAction("switch-audio-output", next);
  //
  // What a shell places a module with, its props, is the shell's own and not a setting; how a
  // module's props and settings go together is the module's to say.
  // ---------------------------------------------------------------------------------------------

  /** The pages of bento's settings a setting can show on, by their id. bento fills this in for
   * editors, from the pages its settings have. */
  export interface SettingsPages {}

  /** The types a setting can have, as engines' have them. */
  export type SettingType =
    | "boolean"
    | "number"
    | "string"
    | "choice"
    | "size"
    | "color"
    | "gradient";

  /** A setting as a module or shell declares it. */
  export interface Setting {
    type: SettingType;
    /** Whether it's a list of values of its type, written as several. */
    list?: boolean;
    /** The values a `choice` takes. */
    choices?: readonly string[];
    /** Its value when the file doesn't set it, as the file writes it: `"50%"` for a size. A
     * size or gradient without one has none. */
    default?: unknown;
    /** What it is, which bento's settings show. */
    description?: string;
    /** The page of bento's settings it shows on. One that names none doesn't show there: it's
     * the module's to change where it shows, like pinned apps, and the file's. */
    page?: keyof SettingsPages;
    /** The group on that page it shows in, by its title, rather than one with the module's
     * name. */
    group?: string;
  }

  /** An argument of an action, which binds write after its name. */
  export interface ActionArg {
    name: string;
    /** Unset, it's any value, and may be left out. */
    type?: "boolean" | "number" | "string" | "choice" | "size" | "workspace" | "monitor";
    /** Its value when it's left out. Arguments without one must be given. */
    default?: boolean | number | string;
    choices?: readonly string[];
    description?: string;
  }

  /** An action as a module or shell declares it. */
  export interface Action {
    description?: string;
    /** Its arguments, in order. */
    args?: readonly ActionArg[];
    /** The key it's bound to while the module runs, written as a bind's key in bento's config,
     * like `"Mod+V"`; a key of modifiers alone, like `"Mod"`, runs it when they're let go with
     * nothing pressed in between. It takes the key over from a bind in the config, the way an
     * engine's binds do, and a bind of its own in the config replaces it. */
    bind?: string;
  }

  /** What a module or shell says about itself. */
  export interface Meta {
    /** Its settings, by name. */
    settings?: Record<string, Setting>;
    /** Its actions, by name, which no other module and none of bento's own has. */
    actions?: Record<string, Action>;
  }

  /** A size, as a setting of type `size` reads: a proportion of the working area, or pixels. */
  export type SizeValue = { proportion: number } | { fixed: number };

  /** A gradient, as a setting of type `gradient` reads and is written. */
  export interface GradientValue {
    from: string;
    to: string;
    angle?: number;
    relativeTo?: "node" | "view";
    in?: string;
  }

  type ScalarOf<T extends SettingType> = T extends "boolean"
    ? boolean
    : T extends "number"
      ? number
      : T extends "size"
        ? SizeValue | null
        : T extends "gradient"
          ? GradientValue | null
          : string;

  /** The value a setting reads as. */
  export type SettingValue<S extends Setting> = S["list"] extends true
    ? ScalarOf<S["type"]>[]
    : ScalarOf<S["type"]>;

  /** The settings `M` declares, by name, with their values. */
  export type Settings<M extends Meta> = {
    readonly [K in keyof M["settings"]]: M["settings"][K] extends Setting
      ? SettingValue<M["settings"][K]>
      : never;
  };

  /** Hands bento what `meta` declares for the module `module` is, which is `import.meta`: its
   * settings and actions. Gives a store of the settings' values, which changes as the file does,
   * and what writes one to the file, as it's written there, like `"50%"` for a size; a setting
   * written as its default leaves the file. A module in place of bento's module of its name has
   * its settings. */
  export function useModule<M extends Meta>(
    module: ImportMeta,
    meta: M,
  ): {
    settings: Settings<M>;
    setSetting<K extends keyof M["settings"] & string>(setting: K, value: unknown): void;
  };

  export function log(...parts: unknown[]): void;

  /** Shows `component` apart from the shell's tree, for what a module shows by itself whichever
   * shell imports it, like a menu: its surfaces go on the outputs like the shell's. It stays until
   * the shell loads again, or what this gives is called. */
  export function mountApart(component: () => JSX.Element): () => void;

  // ---------------------------------------------------------------------------------------------
  // The data program
  //
  // Sound, networks, batteries, notifications and the like come from `bento-shell --data`, a
  // program of its own next to bento, so that nothing it waits on ever holds up a frame.
  // ---------------------------------------------------------------------------------------------

  export interface AudioDevice {
    /** Its name in the sound server, which commands take. */
    name: string;
    label: string;
    icon: string;
    volume: number;
    muted: boolean;
    active: boolean;
  }

  /** The data program's state, by kind. */
  export interface Data {
    /** The default output's volume. */
    audio: { available: boolean; volume: number; muted: boolean };
    "audio-devices": { outputs: AudioDevice[]; inputs: AudioDevice[] };
    network: { connected: boolean; name: string | null; wifiStrength: number | null; icon: string };
    battery: { present: boolean; percentage: number; charging: boolean; icon: string };
    /** Each item's `icon` is an icon's name or a picture's path. */
    /** Apps' status icons. `id` is where one is on the bus, which commands take; `name` is the
     * app's own name for it, like `nm-applet`, which stays the same from one run to the next. */
    tray: {
      items: {
        id: string;
        name: string;
        title: string;
        icon: string | null;
        /** Whether it only has a menu, which a click opens. */
        menu: boolean;
        attention: boolean;
      }[];
    };
    wifi: {
      available: boolean;
      enabled: boolean;
      /** The network being joined, by its name. */
      joining: string | null;
      networks: { ssid: string; strength: number; secure: boolean; known: boolean; active: boolean }[];
    };
    bluetooth: {
      available: boolean;
      powered: boolean;
      discovering: boolean;
      devices: {
        path: string;
        name: string;
        icon: string | null;
        paired: boolean;
        connected: boolean;
        connecting: boolean;
      }[];
    };
    "power-profile": { available: boolean; active: string | null; profiles: string[] };
    media: {
      players: {
        id: string;
        name: string;
        icon: string | null;
        playing: boolean;
        title: string | null;
        artist: string | null;
        art: string | null;
        canNext: boolean;
        canPrevious: boolean;
        canPlay: boolean;
      }[];
    };
    clipboard: {
      available: boolean;
      /** The newest first. */
      entries: ({ id: number; kind: "text"; text: string } | { id: number; kind: "image"; image: string })[];
    };
    brightness: { displays: { id: string; label: string; value: number }[] };
    theme: { dark: boolean };
    weather:
      | { ok: false }
      | {
          ok: true;
          temperature: number;
          feels: number | null;
          humidity: number | null;
          wind: number | null;
          windUnit: string;
          unit: string;
          label: string;
          icon: string;
          days: { date: string; label: string; icon: string; min: number; max: number }[];
        };
  }

  /** The data program's latest of `kind`, as a store, or `undefined` until it says. */
  export function data<K extends keyof Data>(kind: K): Data[K] | undefined;

  export interface Notification {
    id: number;
    app: string;
    /** An icon's name or a picture's path. */
    appIcon: string | null;
    summary: string;
    body: string;
    /** `[key, label]` pairs. */
    actions: [string, string][];
    /** 0 low, 1 normal, 2 critical. */
    urgency: 0 | 1 | 2;
    /** In milliseconds; -1 for the server's choice, 0 never. */
    timeout: number;
    image: string | null;
    transient: boolean;
  }

  /** The data program's events, which aren't state. The notifications showing are told again
   * as `"shown"` to a shell loaded again. `ready` is the data program starting, to tell it what
   * it's asked to do from then on, like where to fetch the weather. */
  export interface DataEvents {
    notification: ({ event: "shown" | "replaced" } & Notification) | { event: "closed"; id: number };
    "wifi-error": { ssid: string; message: string };
    "bluetooth-error": { message: string };
    ready: {};
    /** An app's menu for its status icon, as `tray-menu` asks for it: empty when it has none. */
    "tray-menu": { id: string; menu: TrayMenuItem[] };
  }

  /** An item of an app's menu for its status icon, which `tray-menu-click` picks by its `id`. */
  export interface TrayMenuItem {
    id: number;
    label: string;
    enabled: boolean;
    toggle: "check" | "radio" | null;
    on: boolean;
    separator: boolean;
    children: TrayMenuItem[];
  }
  /** Listens to the data program's events. Listening inside a component, it stops when the
   * component goes. */
  export function onData<K extends keyof DataEvents>(kind: K, fn: (event: DataEvents[K]) => void): void;

  /** What the data program can be told to do. */
  export interface Commands {
    /** The default output's volume. */
    volume: { value: number };
    mute: { value: boolean };
    /** Makes a device the default. */
    device: { kind: "output" | "input"; name: string };
    "device-volume": { kind: "output" | "input"; name: string; value: number };
    "device-mute": { kind: "output" | "input"; name: string; value: boolean };
    dismiss: { id: number };
    expire: { id: number };
    /** Does a notification's action; with `token`, bento hands the app an activation token, to
     * raise its window. */
    invoke: { id: number; key: string; token?: boolean };
    activate: { id: string };
    secondary: { id: string };
    /** Asks for an app's menu for its status icon, which comes as the `tray-menu` event. */
    "tray-menu": { id: string };
    /** Picks an item of an app's menu. */
    "tray-menu-click": { id: string; item: number };
    suspend: {};
    reboot: {};
    "power-off": {};
    brightness: { id: string; value: number };
    "brightness-refresh": {};
    theme: { dark: boolean };
    weather: { latitude?: number; longitude?: number; fahrenheit?: boolean };
    "wifi-enable": { on: boolean };
    "wifi-scan": {};
    "wifi-join": { ssid: string; password?: string };
    "wifi-disconnect": {};
    "wifi-forget": { ssid: string };
    "bluetooth-power": { on: boolean };
    "bluetooth-scan": { on: boolean };
    "bluetooth-connect": { path: string };
    "bluetooth-disconnect": { path: string };
    "bluetooth-forget": { path: string };
    "power-profile": { name: string };
    media: { player: string; action: "PlayPause" | "Next" | "Previous" };
    /** Copies a clipboard entry again. */
    "clipboard-copy": { id: number };
    "clipboard-delete": { id: number };
    "clipboard-clear": {};
  }
  export function command<K extends keyof Commands>(cmd: K, fields: Commands[K]): void;

  // Solid's control flow, which compiled JSX brings in from bento itself.
  export { For, Show, Switch, Match, Repeat, Errored, Loading, Reveal } from "solid-js";

  export namespace JSX {
    type Element = SolidElement;
  }
}

declare module "bento/jsx-runtime" {
  import type { Element as SolidElement } from "solid-js";
  import type { BoxProps, TextProps, ImageProps, InputProps, CopyProps } from "bento";

  export namespace JSX {
    type Element = SolidElement;
    interface ElementChildrenAttribute {
      children: {};
    }
    interface IntrinsicElements {
      /** A flex box: what holds things, and any shape. */
      box: BoxProps;
      /** One run of text, with the text styles it inherits. */
      text: TextProps;
      /** An icon or a picture. */
      image: ImageProps;
      /** A text field: bento keeps what's typed, its caret and selection. */
      input: InputProps;
      /** A live copy of a window, as for previews and switchers. */
      copy: CopyProps;
    }
  }
}

declare module "bento:modules/*";
declare module "bento:stock/modules/*";
declare module "bento:shells/*";
declare module "bento:stock/shells/*";
