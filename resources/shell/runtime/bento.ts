// bento's shell API, and Solid's renderer for bento's scene. `types/bento.d.ts` says what each
// export is for; this is how they work.
//
// Each element is a node in bento's scene, which bento lays out, animates and draws. Solid keeps
// the tree it renders here, in `Node`s; bento's own tree follows it, but for surfaces, which go
// on their output wherever they're rendered, and nodes on their way out, which stay until their
// exit motion settles.

import {
  createComponent,
  createContext,
  createMemo,
  createRenderer,
  createRoot,
  createSignal,
  createStore,
  reconcile,
  flush,
  getOwner,
  mapArray,
  merge,
  onCleanup,
  untrack,
  useContext,
} from "solid-js";
// Shared with engines' prelude, so that engines and shells declare alike.
import { checkActions, checkSettings } from "./declare.js";

const native = (globalThis as any).__bento;

// ------------------------------------------------------------------------------------------------
// Timers, for scripts and for Solid.
// ------------------------------------------------------------------------------------------------

const timers = new Map<number, () => void>();

function startTimer(ms: number, fn: () => void): number {
  const id = native.after(Math.max(0, Number(ms) || 0));
  timers.set(id, fn);
  return id;
}

function stopTimer(id: number | undefined) {
  if (id === undefined || !timers.has(id)) return;
  native.cancel(id);
  timers.delete(id);
}

const global = globalThis as any;
global.setTimeout ??= (fn: (...args: unknown[]) => void, ms = 0, ...args: unknown[]) =>
  startTimer(ms, () => fn(...args));
global.clearTimeout ??= stopTimer;
global.setInterval ??= (fn: (...args: unknown[]) => void, ms = 0, ...args: unknown[]) => {
  const state = { id: 0 };
  const tick = () => {
    state.id = startTimer(ms, tick);
    fn(...args);
  };
  state.id = startTimer(ms, tick);
  intervals.set(state, true);
  return state;
};
global.clearInterval ??= (state: { id: number }) => state && stopTimer(state.id);
global.queueMicrotask ??= (fn: () => void) => Promise.resolve().then(fn);
// What Solid says, like an error in an effect, goes to bento's log.
const say = (...parts: unknown[]) =>
  native.log(parts.map((part) => (part instanceof Error ? `${part}\n${part.stack}` : String(part))).join(" "));
global.console ??= { log: say, info: say, warn: say, error: say, debug: say, trace: say };
const intervals = new WeakMap<object, boolean>();

/** Runs `fn` when the component or effect running now goes, if any is. */
function whenGone(fn: () => void) {
  if (getOwner()) onCleanup(fn);
}

export function after(ms: number, fn: () => void): () => void {
  let id: number | undefined = startTimer(ms, () => {
    id = undefined;
    fn();
  });
  const stop = () => {
    stopTimer(id);
    id = undefined;
  };
  whenGone(stop);
  return stop;
}

export function every(ms: number, fn: () => void): () => void {
  let id: number | undefined;
  const tick = () => {
    id = startTimer(ms, tick);
    fn();
  };
  id = startTimer(ms, tick);
  const stop = () => {
    stopTimer(id);
    id = undefined;
  };
  whenGone(stop);
  return stop;
}

// One timer per pace for every clock that keeps it.
const clocks = new Map<number, () => Date>();

export function now(ms = 1000): () => Date {
  let now = clocks.get(ms);
  if (!now) {
    createRoot(() => {
      const [time, setTime] = createSignal(new Date(), { equals: false });
      // On the pace's mark, so that a second ticks as the wall clock's does.
      const tick = () => {
        setTime(new Date());
        startTimer(ms - (Date.now() % ms), tick);
      };
      startTimer(ms - (Date.now() % ms), tick);
      now = time;
    });
    clocks.set(ms, now!);
  }
  return now!;
}

export function log(...parts: unknown[]) {
  native.log(parts.map((part) => (typeof part === "string" ? part : JSON.stringify(part))).join(" "));
}

// ------------------------------------------------------------------------------------------------
// bento's state, as signals.
// ------------------------------------------------------------------------------------------------

const read = <T>(json: string | undefined, fallback: T): T => {
  try {
    return json ? (JSON.parse(json) ?? fallback) : fallback;
  } catch {
    return fallback;
  }
};

const [outputList, setOutputList] = createSignal<string[]>(read(native.outputs(), []));
export const outputs = outputList;

/** What tells the items of a list in bento's state apart, for its rows to stay as it changes. */
const key = (item: any) =>
  typeof item !== "object" || item === null
    ? item
    : item.id ?? item.ssid ?? item.path ?? item.name ?? item.date ?? item.label;

/** A store of what bento says, its lists' items kept as they change, with whether it said. */
function keyed<T extends object>(initial: T) {
  return createRoot(() => {
    const [store, setStore] = createStore<any>(initial);
    const [said, setSaid] = createSignal(false);
    const set = (value: T) => {
      setStore(reconcile(value, key));
      setSaid(true);
    };
    return [store as T, set, said] as const;
  });
}

const [workspaceMap, setWorkspaceMap] = keyed<Record<string, any[]>>({});
const [fullscreenMap, setFullscreenMap] = keyed<Record<string, boolean>>({});
const [windowList, setWindowList] = keyed<{ list: any[] }>({ list: [] });

export function windows() {
  return windowList.list;
}
const [overviewValue, setOverviewValue] = createSignal(false);
export const overviewOpen = overviewValue;
const [styleValue, setStyleValue] = createSignal<any>(read(native.style(), {}));

export const focusedOutput = createRoot(() =>
  createMemo(() => {
    for (const [output, list] of Object.entries(workspaceMap)) {
      if (list.some((ws: any) => ws.focused)) return output;
    }
    return null;
  }),
);

const [currentValue, setCurrentValue] = createSignal<string | null>(null);
export const currentOutput = currentValue;

export function workspaces(output: string) {
  return workspaceMap[output] ?? [];
}

export function fullscreen(output: string) {
  return Boolean(fullscreenMap[output]);
}

export function minimap(output: string) {
  outputs();
  const size = outputSize(output);
  const shown = workspaces(output).find((ws: any) => ws.active);
  if (!size || !shown) return null;
  const tiles = windows()
    .filter((win) => win.workspace === shown.id && win.rect)
    .map(({ id, rect, active, floating }) =>
      ({ id, x: rect.x, y: rect.y, w: rect.width, h: rect.height, active, floating }));
  return { view: [size.width, size.height], tiles };
}

export function outputSize(output: string) {
  return read<{ width: number; height: number } | null>(native.outputSize(output), null);
}

const STYLE_DEFAULTS = {
  gaps: 8, cornerRadius: 12, animations: true, slowdown: 1, response: 0.42, dark: true,
};

export const bentoStyle = createRoot(() =>
  createMemo(() => {
    const { config: _config, ...style } = styleValue() ?? {};
    return { ...STYLE_DEFAULTS, ...style, animations: style.animations ?? true };
  }, { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) }),
);

const CONFIG_DEFAULTS = { location: null };

export const config = createRoot(() =>
  createMemo(() => ({ ...CONFIG_DEFAULTS, ...(styleValue()?.config ?? {}) }), {
    equals: (a, b) => JSON.stringify(a) === JSON.stringify(b),
  }),
);

export function paced(ms: number) {
  const { animations, slowdown } = untrack(bentoStyle);
  return animations ? ms * (slowdown ?? 1) : 0;
}

/** How long bento's springs take to come most of the way, in milliseconds. */
export function settling() {
  return (untrack(bentoStyle).response ?? 0.42) * 1000;
}

// ------------------------------------------------------------------------------------------------
// Doing things.
// ------------------------------------------------------------------------------------------------

export function action(
  name: string,
  args: unknown[] = [],
  { properties = {}, window, workspace }: { properties?: object; window?: number; workspace?: number } = {},
) {
  native.action(JSON.stringify({ name, args, properties, window, workspace }));
}

export function apps() {
  return read(native.apps(), []);
}

export function launch(id: string) {
  native.launch(id);
}

export function preload(icon: string, size = 16) {
  native.preload(String(icon), size);
}

export function command(cmd: string, fields: object = {}) {
  native.data(JSON.stringify({ cmd, ...fields }));
}

const running = new Map<number, (code: number) => void>();
let nextRun = 0;

export function run(commandLine: string): Promise<number> {
  const id = nextRun++;
  return new Promise((resolve) => {
    running.set(id, resolve);
    command("run", { id, command: commandLine });
  });
}

export function notify(summary: string, body = "", icon = "", { transient = false } = {}) {
  command("notify", { summary, body, icon, transient });
}

export function stored<T>(key: string, fallback: T): [() => T, (value: T) => void] {
  const [value, setValue] = createSignal<T>(read<any>(native.stored(), {})[key] ?? fallback, {
    equals: false,
  });
  const set = (next: T) => {
    native.store(JSON.stringify({ ...read<any>(native.stored(), {}), [key]: next }));
    setValue(() => next);
  };
  return [value, set];
}

// The settings that modules and shells declare in their `meta`, by the module's name, each a
// store that the settings file keeps up to date.
const settingStores = new Map<string, (values: any) => void>();

export function useModule(
  module: { name: string },
  meta: { settings?: Record<string, any>; actions?: Record<string, any> },
) {
  const name = module.name;
  const settings = checkSettings("setting", meta.settings);
  const actions = checkActions(meta.actions);
  const declared = native.declare(name, JSON.stringify(settings), JSON.stringify(actions));
  const [values, setValues] = keyed<any>(read(declared, {}));
  settingStores.set(name, setValues);
  const setSetting = (setting: string, value: unknown) => {
    // bento checks it, writes it to the file, and says what the settings are now.
    setValues(read(native.setSetting(name, setting, JSON.stringify(value ?? null)), {}));
  };
  return { settings: values, setSetting };
}

// What listens to the shell's actions, by their names.
const actionListeners = new Map<string, Set<(event: any) => void>>();

function listen<T>(map: Map<string, Set<T>>, name: string, fn: T) {
  if (!map.has(name)) map.set(name, new Set());
  map.get(name)!.add(fn);
  whenGone(() => map.get(name)?.delete(fn));
}

export function onAction(
  name: string,
  fn: (event: { output: string | null; args: string[] }) => void,
) {
  listen(actionListeners, name, fn);
}

// The data program's state, as a store each, and its events.
const DATA_EVENTS = new Set([
  "notification", "wifi-error", "bluetooth-error", "ready", "tray-menu", "files",
]);
const dataStores = new Map<string, readonly [any, (value: any) => void, () => boolean]>();
const dataListeners = new Map<string, Set<(event: any) => void>>();

function dataStore(kind: string) {
  if (!dataStores.has(kind)) dataStores.set(kind, keyed<any>({}));
  return dataStores.get(kind)!;
}

export function data(kind: string) {
  const [store, , said] = dataStore(kind);
  return said() ? store : undefined;
}

export function onData(kind: string, fn: (event: any) => void) {
  listen(dataListeners, kind, fn);
}

// ------------------------------------------------------------------------------------------------
// Styles.
// ------------------------------------------------------------------------------------------------

export function css<T>(styles: T): T {
  return styles;
}

const STATES = [":hover", ":active", ":focus"] as const;
// A pose: what `enter`, `exit` and `jump` can start or end at, what's moved by springs alone.
const LOOKS = ["opacity", "scale", "blur", "translate", "drawSize", "borderRadius"];

/** One style from a style prop: a list's styles, later ones winning, falsy ones skipped. */
function flatten(style: any, into: Record<string, any> = {}): Record<string, any> {
  if (!style) return into;
  if (Array.isArray(style)) {
    for (const one of style) flatten(one, into);
    return into;
  }
  for (const [key, value] of Object.entries(style)) {
    // Left undefined, a property isn't set, and doesn't take an earlier style's place.
    if (value === undefined) continue;
    if (STATES.includes(key as any)) {
      into[key] = flatten(value, { ...(into[key] ?? {}) });
    } else {
      into[key] = value;
    }
  }
  return into;
}

function looks(style: any) {
  const out: Record<string, any> = {};
  for (const key of LOOKS) if (style?.[key] !== undefined) out[key] = style[key];
  if (style?.transition !== undefined) out.transition = style.transition;
  return out;
}

const same = (a: unknown, b: unknown) =>
  a === b || (typeof a === "object" && JSON.stringify(a) === JSON.stringify(b));

// ------------------------------------------------------------------------------------------------
// Nodes.
// ------------------------------------------------------------------------------------------------

const HANDLERS: Record<string, string> = {
  onClick: "click", onMiddleClick: "click", onRightClick: "click", onPress: "click",
  onRelease: "click", onDrag: "drag", onHover: "hover", onScroll: "scroll", onNear: "near",
  onRest: "settle", onLayout: "layout", onScrolled: "scrolled",
};
const CLICKS: Record<number, string> = { 1: "onClick", 2: "onMiddleClick", 3: "onRightClick" };

/** The nodes there are, by their id in bento's scene. */
const nodes = new Map<number, Node>();

class Node {
  id = -1;
  tag: string;
  parent: Node | null = null;
  children: Node[] = [];
  /** What bento's scene holds in it, in order: its children, and those on their way out. */
  held: Node[] = [];
  handlers: Record<string, ((event: any) => void) | undefined> = {};
  style: Record<string, any> = {};
  /** What's set in bento's scene now. */
  sent: Record<string, any> = {};
  /** Props of its own that aren't its style, like an image's icon. */
  own: Record<string, any> = {};
  hovered = false;
  pressed = false;
  focused = false;
  enter: any = null;
  exit: any = null;
  entered = false;
  leaving: (() => void) | null = null;
  /** For a surface, the size it keeps while what it held goes, and how many are going: emptied,
   * it would shrink to nothing and move, taking them with it. */
  kept: { width: number; height: number; going: number } | null = null;
  /** Whether what made it is gone, so that it goes once it's taken out. */
  disposed = false;
  /** Whether it's in bento's scene, in something. */
  attached = false;
  /** Whether it was sent to a pose: its looks go back to its style, or to their defaults where
   * its style doesn't say, the next time it's updated. */
  posed = false;
  /** Whether it's gone from bento's scene for good. */
  dead = false;
  /** Whether it's on its way to its exit pose to be hidden. */
  hiding = false;
  /** Set while it's hidden once its exit pose is reached. */
  exited = false;
  stopHiding: (() => void) | null = null;
  surface: SurfaceState | null = null;
  input: InputState | null = null;

  constructor(tag: string, kind: string, props: Record<string, any> = {}) {
    this.tag = tag;
    if (tag === "#world") return;
    this.id = native.create(kind, JSON.stringify(props));
    nodes.set(this.id, this);
    this.sent = { ...props };
  }

  /** Sends what changed of its style, with its states, and its handlers, to bento. Hidden
   * with `display: "none"`, it goes to its `exit` pose first, and shown again, it comes from
   * its `enter` pose, as when it's added and taken away. */
  update() {
    if (this.id < 0 || this.dead) return;
    const style: Record<string, any> = { ...this.style };
    for (const state of STATES) delete style[state];
    if (this.hovered) Object.assign(style, this.style[":hover"]);
    if (this.pressed) Object.assign(style, this.style[":active"]);
    if (this.focused) Object.assign(style, this.style[":focus"]);
    const events = new Set<string>();
    for (const [name, handler] of Object.entries(this.handlers)) {
      if (handler && HANDLERS[name]) events.add(HANDLERS[name]);
    }
    if (this.style[":hover"]) events.add("hover");
    if (this.style[":active"]) events.add("click");
    if (this.leaving || this.hiding || this.exit) events.add("settle");
    const next: Record<string, any> = { ...style, ...this.own, events: [...events].sort() };
    if (this.kept) {
      const at = (value: unknown, least: number) =>
        typeof value === "number" && value > least ? value : least;
      next.minWidth = at(next.minWidth, this.kept.width);
      next.minHeight = at(next.minHeight, this.kept.height);
    }
    // What a click does something on shows a hand, unless its style says how the pointer looks.
    const clicks = this.handlers.onClick || this.handlers.onMiddleClick || this.handlers.onRightClick;
    if (clicks && next.cursor === undefined) next.cursor = "pointer";
    // What's undefined isn't set: it goes back to its default, as it would left out.
    for (const key of Object.keys(next)) if (next[key] === undefined) delete next[key];
    const wasShown = this.sent.display !== "none";
    const shown = next.display !== "none";
    if (
      wasShown && !shown && this.exit && this.attached && !this.exited
      && untrack(bentoStyle).animations
    ) {
      // It stays until its exit pose is reached.
      if (!this.hiding) {
        this.hiding = true;
        const pose = looks(flatten(this.exit));
        native.set(this.id, JSON.stringify(pose));
        Object.assign(this.sent, pose);
        // What it holds goes to its own exit poses with it.
        for (const child of this.children) {
          walk(child, (one) => {
            if (!one.exit || one.sent.display === "none") return;
            const inner = looks(flatten(one.exit));
            native.set(one.id, JSON.stringify(inner));
            one.posed = true;
          });
        }
        const timer = startTimer(settling() * 3, () => this.hidden());
        this.stopHiding = () => stopTimer(timer);
      }
      next.display = this.sent.display ?? "flex";
      for (const key of LOOKS) if (key in this.sent) next[key] = this.sent[key];
      if ("transition" in this.sent) next.transition = this.sent.transition;
    } else if (this.hiding && shown) {
      // Shown again on its way out, it turns around from where it is.
      this.hiding = false;
      this.stopHiding?.();
      this.posed = true;
      for (const child of this.children) {
        walk(child, (one) => {
          if (one.exit) one.update();
        });
      }
    } else if (!wasShown && shown && this.entered) {
      // It and what it holds come from their enter poses.
      const poses: [number, any][] = [];
      if (this.enter) poses.push([this.id, looks(flatten(this.enter))]);
      for (const child of this.children) {
        walk(child, (one) => {
          // Their looks go back to their styles, from their enter poses.
          if (one.exit) one.update();
          if (one.enter && one.id >= 0) poses.push([one.id, looks(flatten(one.enter))]);
        });
      }
      if (poses.length) {
        queueMicrotask(() => {
          for (const [id, pose] of poses) native.set(id, JSON.stringify({ jump: pose }));
        });
      }
    }
    const changes: Record<string, any> = {};
    if (this.posed && !exiting(this)) {
      this.posed = false;
      for (const key of [...LOOKS, "transition"]) {
        changes[key] = next[key] ?? null;
        delete this.sent[key];
      }
    }
    for (const key of Object.keys(this.sent)) {
      if (!(key in next) && this.sent[key] !== null) changes[key] = null;
    }
    for (const [key, value] of Object.entries(next)) {
      if (!same(this.sent[key], value)) changes[key] = value;
    }
    if (Object.keys(changes).length === 0) return;
    for (const [key, value] of Object.entries(changes)) {
      if (value === null) delete this.sent[key];
      else this.sent[key] = value;
    }
    native.set(this.id, JSON.stringify(changes));
  }

  /** Its exit pose was reached: it's hidden, as its style says. */
  hidden() {
    if (!this.hiding) return;
    this.hiding = false;
    this.stopHiding?.();
    this.exited = true;
    this.update();
    this.exited = false;
  }

  layout() {
    const rect = read<number[] | null>(native.layout(this.id), null);
    return rect && { x: rect[0], y: rect[1], width: rect[2], height: rect[3] };
  }

  frame() {
    const rect = read<number[] | null>(native.frame(this.id), null);
    return rect && { x: rect[0], y: rect[1], width: rect[2], height: rect[3] };
  }

  jump(from: any) {
    if (this.dead) return;
    native.set(this.id, JSON.stringify({ jump: looks(flatten(from)) }));
  }

  // One whose style scrolls it.
  scrollTo(top: number) {
    if (!this.dead) native.scrollTo(this.id, top);
  }

  // An input's.
  focus() {
    if (this.input) focusInput(this);
  }

  blur() {
    if (focusedInput === this) focusInput(null);
  }

  get text() {
    return this.input?.current ?? "";
  }

  setText(text: string) {
    this.input?.setText(text);
  }
}

/** Whether `node`, or what holds it, is on its way to its exit pose, which it stays at. */
function exiting(node: Node | null): boolean {
  for (let at = node; at; at = at.parent) if (at.leaving || at.hiding) return true;
  return false;
}

/** The surface `node` is in, if any. */
function surfaceOf(node: Node | null): Node | null {
  while (node && !node.surface) node = node.parent;
  return node;
}

/** Calls `fn` on `node` and everything Solid has in it. */
function walk(node: Node, fn: (node: Node) => void) {
  fn(node);
  for (const child of node.children) walk(child, fn);
}

// ------------------------------------------------------------------------------------------------
// Surfaces.
// ------------------------------------------------------------------------------------------------

interface SurfaceState {
  output: string | null;
  keyboard: "none" | "on-demand" | "exclusive";
  onKey?: (event: any) => boolean | void;
  onEscape?: () => void;
  onOutside?: () => void;
  mounted: boolean;
  /** When it was last shown or pressed, to tell which holds the keyboard. */
  since: number;
}

const surfaces = new Set<Node>();
let order = 0;
let pressedSurface: Node | null = null;

/** Puts surface `node` on its output. */
function mount(node: Node) {
  const surface = node.surface!;
  const root = surface.output === null ? -1 : native.root(surface.output);
  if (root < 0) {
    unmount(node);
    return;
  }
  native.append(root, node.id, -1);
  node.attached = true;
  if (!surface.mounted) {
    surface.mounted = true;
    surface.since = ++order;
    surfaces.add(node);
    enter(node);
  }
  updateKeyboard();
}

/** Takes surface `node` off its output, after its exit motion. */
function unmount(node: Node) {
  const surface = node.surface!;
  if (!surface.mounted) return;
  surface.mounted = false;
  surfaces.delete(node);
  if (pressedSurface === node) pressedSurface = null;
  leave(node);
  updateKeyboard();
}

/** The surface that holds the keyboard: the newest one that takes it for as long as it shows,
 * or the last one pressed that takes it on demand. */
const LAYERS = ["background", "bottom", "top", "overlay"];

/** Where a surface is from the back to the front, as bento draws them: its layer, which side
 * of other programs' surfaces, its `z`, and then which came last. */
function rank(node: Node): number[] {
  const layer = LAYERS.indexOf(node.own.layer ?? "top");
  const z = node.own.z ?? 1;
  return [layer < 0 ? 2 : layer, z >= 0 ? 1 : 0, z, node.surface!.since];
}

function above(a: Node, b: Node) {
  const [ra, rb] = [rank(a), rank(b)];
  for (let i = 0; i < ra.length; i++) if (ra[i] !== rb[i]) return ra[i] > rb[i];
  return false;
}

function keyboardHolder(): Node | null {
  let holder: Node | null = null;
  for (const node of surfaces) {
    if (node.surface!.keyboard === "exclusive" && (!holder || above(node, holder))) {
      holder = node;
    }
  }
  if (holder) return holder;
  return pressedSurface?.surface?.keyboard === "on-demand" && pressedSurface.surface.mounted
    ? pressedSurface
    : null;
}

let holding = false;

function updateKeyboard() {
  const holder = keyboardHolder();
  if (focusedInput && surfaceOf(focusedInput) !== holder) focusInput(null);
  if (holding !== Boolean(holder)) {
    holding = Boolean(holder);
    native.keyboard(holding);
  }
}

const OutputContext = createContext<string | null>(null);

export function useOutput(): string {
  const output = useContext(OutputContext);
  if (output === null || output === undefined) {
    throw new Error("useOutput() is only for what's in a <Surface>");
  }
  return output;
}

function surfaceOn(name: string, props: any) {
  return createComponent(OutputContext as any, {
    value: name,
    get children() {
      const node = renderer.createElement("surface");
      // What it holds is inserted here: a merged props' children aren't its own to spread.
      renderer.spread(node, merge(props, { output: name }), true);
      renderer.insert(node, () => props.children);
      return node;
    },
  });
}

export function Surface(props: any) {
  return createMemo(() => {
    const where = props.output ?? "all";
    if (where === "all") return mapArray(outputs, (name: string) => surfaceOn(name, props));
    if (where === "focused") {
      const name = focusedOutput() ?? outputs()[0];
      return name ? surfaceOn(name, props) : null;
    }
    return surfaceOn(where, props);
  }) as any;
}

// ------------------------------------------------------------------------------------------------
// Coming and going.
// ------------------------------------------------------------------------------------------------

/** Starts `node` from its `enter` looks, the first time it's shown. */
function enter(node: Node) {
  if (node.entered) return;
  node.entered = true;
  if (!node.enter) return;
  const from = looks(flatten(node.enter));
  // Once the props that came with it are set, so that it moves from there to them.
  queueMicrotask(() => native.set(node.id, JSON.stringify({ jump: from })));
}

/** Takes `node` out of bento's scene, after its exit motion if it has one, from `from`, what held
 * it, which it's no longer in. */
function leave(node: Node, from: Node | null = null) {
  const holder = node.surface ? null : from;
  // Whether its surface keeps its size for it while it goes.
  let keeping = false;
  const done = () => {
    node.leaving = null;
    if (holder) holder.held = holder.held.filter((one) => one !== node);
    if (keeping && holder?.kept && --holder.kept.going === 0) {
      holder.kept = null;
      holder.update();
    }
    if (node.dead) return;
    native.detach(node.id);
    node.attached = false;
    if (node.disposed) destroy(node);
  };
  const animated = untrack(bentoStyle).animations;
  const exit = node.exit && animated ? looks(flatten(node.exit)) : null;
  // What it holds goes to its own exit poses with it.
  let inner = false;
  if (animated && node.attached) {
    for (const child of node.children) {
      walk(child, (one) => {
        if (one.exit && one.attached && one.sent.display !== "none") {
          native.set(one.id, JSON.stringify(looks(flatten(one.exit))));
          one.posed = true;
          inner = true;
        }
      });
    }
  }
  if ((!exit || Object.keys(exit).length === 0) && !inner) {
    done();
    return;
  }
  if (!node.attached) {
    done();
    return;
  }
  // It stays where it is, out of the flow, while it goes.
  const place = node.layout();
  const around = holder?.layout();
  if (!node.surface && place && around) {
    Object.assign(node.own, {
      position: "absolute", left: place.x - around.x, top: place.y - around.y,
      width: place.width, height: place.height,
    });
    // A surface keeps its size while it goes, so it stays where it is, and the surface with it.
    if (holder?.surface) {
      keeping = true;
      const kept = holder.kept ?? { width: 0, height: 0, going: 0 };
      holder.kept = {
        width: Math.max(kept.width, around.width),
        height: Math.max(kept.height, around.height),
        going: kept.going + 1,
      };
      holder.update();
    }
  }
  let stop: (() => void) | null = null;
  node.leaving = () => {
    stop?.();
    done();
  };
  node.update();
  if (exit && Object.keys(exit).length > 0) {
    native.set(node.id, JSON.stringify(exit));
    node.posed = true;
  }
  // It goes when its motion settles, or once it should have; with only what it holds going
  // somewhere, once that has.
  const timer = startTimer(settling() * (exit ? 3 : 1.2), () => node.leaving?.());
  stop = () => stopTimer(timer);
}

/** Brings back a node that was on its way out. */
function comeBack(node: Node) {
  if (!node.leaving) return;
  node.leaving = null;
  for (const key of ["position", "left", "top", "width", "height"]) delete node.own[key];
  // Its looks go back to its style from wherever they are.
  node.posed = true;
  node.update();
}

/** Lets `node` and what it holds go from bento's scene. */
function destroy(node: Node) {
  // Gone already: bento may have given its id to another node since.
  if (node.id < 0 || nodes.get(node.id) !== node) return;
  native.remove(node.id);
  const forget = (one: Node) => {
    if (nodes.get(one.id) === one) nodes.delete(one.id);
    // Its id may go to another node now, which nothing done to this one may touch.
    one.dead = true;
    if (focusedInput === one) focusInput(null);
    for (const child of one.held) forget(child);
  };
  forget(node);
}

/** What holds `node` in bento's scene: its parent, unless that's the world. */
function nativeParent(node: Node): Node | null {
  const parent = node.parent;
  return parent && parent.id >= 0 ? parent : null;
}

// ------------------------------------------------------------------------------------------------
// The renderer.
// ------------------------------------------------------------------------------------------------

const KINDS: Record<string, string> = {
  box: "box", text: "text", image: "image", input: "box", surface: "surface", copy: "copy",
};

/** Sets a `<text>`'s words from the strings and numbers in it, as one run of text. */
function joinText(node: Node) {
  node.own.text = node.children
    .filter((child) => child.tag === "#text")
    .map((child) => child.own.text ?? "")
    .join("");
  node.update();
}
const SURFACE_PROPS = ["layer", "z", "anchor", "reserve"];
const IMAGE_PROPS = ["icon", "src", "size"];

function createNode(tag: string, props?: Record<string, any>): Node {
  const kind = KINDS[tag];
  if (!kind) {
    throw new Error(`there's no <${tag}>: bento's are <box>, <text>, <image>, <input>, <copy> and <surface>`);
  }
  const node = new Node(tag, kind, tag === "image" ? { size: 16, flexShrink: 0 } : {});
  if (tag === "image") node.own = { size: 16, flexShrink: 0 };
  if (tag === "surface") {
    node.surface = {
      output: null, keyboard: "none", mounted: false, since: 0,
    };
  }
  if (tag === "input") makeInput(node);
  // It goes once what made it is gone, and it's out of the scene.
  whenGone(() => {
    node.disposed = true;
    if (!node.attached && !node.leaving) destroy(node);
  });
  if (props) for (const [name, value] of Object.entries(props)) setProperty(node, name, value);
  return node;
}

function setProperty(node: Node, name: string, value: any, _prev?: any) {
  if (name === "style") {
    node.style = flatten(value);
    node.update();
  } else if (name === "enter") {
    node.enter = value;
  } else if (name === "exit") {
    node.exit = value;
    node.update();
  } else if (name in HANDLERS) {
    node.handlers[name] = value;
    node.update();
  } else if (node.surface && name === "output") {
    node.surface.output = value ?? null;
    if (node.surface.mounted || node.parent) mount(node);
  } else if (node.surface && name === "keyboard") {
    node.surface.keyboard = value ?? "none";
    updateKeyboard();
  } else if (node.surface && (name === "onKey" || name === "onEscape" || name === "onOutside")) {
    (node.surface as any)[name] = value;
  } else if (node.surface && name === "attach") {
    // Beside a node, by its id in bento's scene, or a point on the output.
    const to = value?.to;
    const where = to instanceof Node
      ? (to.id >= 0 ? { node: to.id } : null)
      : typeof to?.x === "number" && typeof to?.y === "number"
        ? { point: [to.x, to.y] }
        : null;
    node.own.attach = where
      ? { ...where, side: value.side ?? "bottom", align: value.align ?? "start", gap: value.gap ?? 0 }
      : null;
    node.update();
  } else if (node.surface && SURFACE_PROPS.includes(name)) {
    node.own[name] = value;
    node.update();
  } else if (node.tag === "image" && IMAGE_PROPS.includes(name)) {
    // An icon's name, or a picture's path, which tray icons and notifications give alike.
    if (name === "icon" && typeof value === "string" && value.startsWith("/")) {
      delete node.own.icon;
      node.own.src = value;
    } else {
      if (name === "icon") delete node.own.src;
      node.own[name] = value;
    }
    node.update();
  } else if (node.tag === "copy" && name === "window") {
    node.own.window = value ?? null;
    node.update();
  } else if (node.input) {
    node.input.set(name, value);
  }
}

function insertChild(parent: Node, node: Node, anchor?: Node | null) {
  if (node.parent) {
    node.parent.children = node.parent.children.filter((one) => one !== node);
  }
  const at = anchor ? parent.children.indexOf(anchor) : -1;
  if (at >= 0) parent.children.splice(at, 0, node);
  else parent.children.push(node);
  node.parent = parent;
  comeBack(node);

  if (parent.tag === "text") {
    // A `<text>` is one run of its strings, not a box of them.
    if (node.tag === "#text") joinText(parent);
    else log(`a <${node.tag}> in a <text> doesn't show: only strings and numbers do`);
    return;
  }
  if (node.surface) {
    mount(node);
  } else if (parent.id >= 0) {
    parent.held = parent.held.filter((one) => one !== node);
    const index = anchor ? parent.held.indexOf(anchor) : -1;
    if (index >= 0) parent.held.splice(index, 0, node);
    else parent.held.push(node);
    // Before what comes after it, by id, rather than at an index: bento holds what's on its way
    // out, and doesn't hold some of what the script has, so their places differ.
    const after = parent.children.slice(parent.children.indexOf(node) + 1).map((one) => one.id);
    // A bento built before it had `insertBefore` places it by index.
    if (native.insertBefore) native.insertBefore(parent.id, node.id, JSON.stringify(after));
    else native.append(parent.id, node.id, index >= 0 ? index : -1);
    node.attached = true;
    enter(node);
  } else {
    log(`a <${node.tag}> was put outside any <Surface>, where it can't show`);
  }
  // Surfaces rendered in it go on their outputs again.
  for (const child of node.children) {
    walk(child, (one) => {
      if (one.surface && !one.surface.mounted) mount(one);
    });
  }
}

function removeChild(parent: Node, node: Node) {
  parent.children = parent.children.filter((one) => one !== node);
  node.parent = null;
  if (parent.tag === "text") {
    joinText(parent);
    return;
  }
  // Surfaces rendered in it go with it.
  for (const child of node.children) {
    walk(child, (one) => {
      if (one.surface) unmount(one);
    });
  }
  if (node.surface) unmount(node);
  else if (parent.id >= 0) leave(node, parent);
}

function makeTextNode(value: unknown): Node {
  const node = new Node("#text", "text", { text: String(value) });
  node.own = { text: String(value), display: value === "" ? "none" : "flex" };
  node.update();
  whenGone(() => {
    node.disposed = true;
    if (!node.attached && !node.leaving) destroy(node);
  });
  return node;
}

const renderer = createRenderer<Node>({
  createElement: createNode,
  createTextNode: makeTextNode,
  replaceText(node: Node, value: string) {
    node.own = { text: String(value), display: value === "" ? "none" : "flex" };
    if (node.parent?.tag === "text") joinText(node.parent);
    else node.update();
  },
  isTextNode: (node: Node) => node.tag === "#text",
  setProperty,
  insertNode: insertChild,
  removeNode: removeChild,
  getParentNode: (node: Node) => node.parent ?? undefined,
  getFirstChild: (node: Node) => node.children[0],
  getNextSibling: (node: Node) => {
    const siblings = node.parent?.children;
    if (!siblings) return undefined;
    return siblings[siblings.indexOf(node) + 1];
  },
} as any) as any;

// What Solid's compiler has compiled JSX import, and the control flow it brings in itself.
export const {
  render,
  effect,
  memo,
  createElement,
  createTextNode,
  insertNode,
  insert,
  spread,
  setProp,
  mergeProps,
  applyRef,
  ref,
} = renderer;
export { createComponent };
export { For, Show, Switch, Match, Repeat, Errored, Loading, Reveal } from "solid-js";

// ------------------------------------------------------------------------------------------------
// Inputs: bento keeps what's typed, its caret and selection; how it looks is here.
// ------------------------------------------------------------------------------------------------

interface InputState {
  current: string;
  set(name: string, value: any): void;
  setText(text: string): void;
  key(event: any): boolean;
  focusChanged(focused: boolean): void;
}

let focusedInput: Node | null = null;

function focusInput(node: Node | null) {
  if (focusedInput === node) return;
  const before = focusedInput;
  focusedInput = node;
  if (before?.input) {
    before.focused = false;
    before.input.focusChanged(false);
    before.update();
  }
  if (node?.input) {
    node.focused = true;
    node.input.focusChanged(true);
    node.update();
  }
}

function makeInput(node: Node) {
  // The input's own box holds what's typed, the placeholder, the caret and the selection.
  node.own = { flexGrow: 1, flexShrink: 1, minWidth: 0, cursor: "text" };
  const field = native.create("input", "{}");
  const hint = native.create("text", JSON.stringify({ position: "absolute", top: 0, left: 0 }));
  const caret = native.create("box", "{}");
  native.append(node.id, field, -1);
  native.append(node.id, hint, -1);
  native.append(node.id, caret, -1);
  let marks: number[] = [];
  let placeholder = "";
  let looks = { placeholder: {} as any, caret: {} as any, selection: {} as any };
  let onInput: ((text: string) => void) | undefined;
  let onSubmit: ((text: string) => void) | undefined;
  let onKey: ((event: any) => boolean | void) | undefined;
  let onBeforeKey: ((event: any) => boolean | void) | undefined;
  let blink: number | undefined;
  let lastTyped = 0;

  const showHint = (text: string) => {
    native.set(hint, JSON.stringify({
      text: placeholder, color: "fg/0.45", opacity: text ? 0 : 1, ...looks.placeholder,
    }));
  };

  const show = (json: string | undefined) => {
    const state = read<any>(json, null);
    if (!state) return null;
    const [x, y, , h] = state.caretRect;
    // Moved, the caret glides there at bento's pace; typing, as fast as the typing goes.
    const typed = state.text !== input.current;
    let transition: number | null = null;
    if (typed) {
      const now = Date.now();
      transition = Math.min(Math.max((now - lastTyped) / 2000, 0.04), settling() / 1000);
      lastTyped = now;
    }
    native.set(caret, JSON.stringify({
      position: "absolute", left: x, top: y, width: 1.5, height: h, background: "fg",
      borderRadius: 1, opacity: node.focused ? 1 : 0, transition, ...looks.caret,
    }));
    showHint(state.text);
    for (const mark of marks) native.remove(mark);
    marks = state.selection.map(([sx, sy, sw, sh]: number[]) => {
      const mark = native.create("box", JSON.stringify({
        position: "absolute", left: sx, top: sy, width: sw, height: sh,
        background: "accent/0.35", animate: false, ...looks.selection,
      }));
      native.append(node.id, mark, 0);
      return mark;
    });
    if (typed) {
      input.current = state.text;
      onInput?.(input.current);
    }
    // The caret stays lit while typing, and blinks once it stops.
    stopTimer(blink);
    blink = undefined;
    if (node.focused) {
      let lit = true;
      const tick = () => {
        lit = !lit;
        native.set(caret, JSON.stringify({ opacity: lit ? 1 : 0 }));
        blink = startTimer(530, tick);
      };
      blink = startTimer(530, tick);
    }
    return state;
  };

  const input: InputState = {
    current: "",
    set(name, value) {
      if (name === "value") {
        input.setText(value ?? "");
      } else if (name === "placeholder") {
        placeholder = value ?? "";
        showHint(input.current);
      } else if (name === "onInput") onInput = value;
      else if (name === "onSubmit") onSubmit = value;
      else if (name === "onKey") onKey = value;
      else if (name === "onBeforeKey") onBeforeKey = value;
      else if (name === "autofocus") {
        if (value) queueMicrotask(() => focusInput(node));
      } else if (name === "placeholderStyle") {
        looks.placeholder = flatten(value);
        showHint(input.current);
      } else if (name === "caretStyle") {
        looks.caret = flatten(value);
      } else if (name === "selectionStyle") {
        looks.selection = flatten(value);
      }
    },
    setText(text) {
      native.set(field, JSON.stringify({ text: String(text) }));
      input.current = String(text);
      show(native.edit(field, "{}"));
    },
    key(event) {
      if (!event.pressed) return false;
      if (onBeforeKey?.(event) === true) return true;
      if (event.key === "Return" || event.key === "KP_Enter") {
        onSubmit?.(input.current);
        return true;
      }
      const state = show(native.edit(field, JSON.stringify(event)));
      if (state?.changed) return true;
      return onKey?.(event) === true;
    },
    focusChanged(focused) {
      if (!focused) {
        stopTimer(blink);
        blink = undefined;
        native.set(caret, JSON.stringify({ opacity: 0 }));
      } else {
        show(native.edit(field, "{}"));
      }
    },
  };
  node.input = input;
  node.handlers.onPress = (event: any) => {
    focusInput(node);
    show(native.edit(field, JSON.stringify({ point: [event.x, event.y] })));
  };
  node.update();
  show(native.edit(field, "{}"));
}

// ------------------------------------------------------------------------------------------------
// bento's events.
// ------------------------------------------------------------------------------------------------

const world = new Node("#world", "");

export function __mount(Shell: any) {
  render(() => createComponent(Shell, {}), world);
  flush();
}

export function mountApart(component: () => any): () => void {
  // A world of its own, whose surfaces go on the outputs like the shell's.
  const apart = new Node("#world", "");
  return render(component, apart);
}

function pointer(node: Node | undefined, name: string, event: any) {
  const handler = node?.handlers[name];
  if (!node || !handler) return;
  // Where on it, too, and how big it's drawn.
  if (typeof event?.x === "number" && name !== "onLayout") {
    const frame = node.frame();
    if (frame) {
      event.offsetX = event.x - frame.x;
      event.offsetY = event.y - frame.y;
      event.frame = frame;
    }
  }
  handler(event);
}

export function __dispatch(json: string) {
  try {
    handle(JSON.parse(json));
  } finally {
    // What the event changed shows before bento draws again.
    flush();
  }
}

function handle(event: any) {
  const node = event.id !== undefined ? nodes.get(event.id) : undefined;
  switch (event.type) {
    case "timer": {
      const fn = timers.get(event.id);
      timers.delete(event.id);
      fn?.();
      return;
    }
    case "press":
      if (node) {
        node.pressed = true;
        node.update();
      }
      pointer(node, "onPress", event);
      return;
    case "release":
      if (node) {
        node.pressed = false;
        node.update();
      }
      pointer(node, "onRelease", event);
      return;
    case "click":
      pointer(node, CLICKS[event.button] ?? "onClick", event);
      return;
    case "hover":
      if (node) {
        node.hovered = event.inside;
        node.update();
      }
      pointer(node, "onHover", event);
      return;
    case "settled":
      if (node?.leaving) node.leaving();
      else if (node?.hiding) node.hidden();
      pointer(node, "onRest", event);
      return;
    case "scrolled":
      pointer(node, "onScrolled", { top: event.top, reach: event.reach, height: event.height });
      return;
    case "layout":
      pointer(node, "onLayout", {
        x: event.x, y: event.y, width: event.width, height: event.height,
      });
      return;
    case "near":
      pointer(node, "onNear", event);
      return;
    case "drag":
      pointer(node, "onDrag", event);
      return;
    case "scroll":
      pointer(node, "onScroll", event);
      return;
    case "down": {
      // A press anywhere: on a surface, or off the shell.
      const on = event.surface === null ? null : nodes.get(event.surface) ?? null;
      for (const one of [...surfaces]) {
        if (one !== on) one.surface!.onOutside?.();
      }
      if (on?.surface?.keyboard === "on-demand") {
        pressedSurface = on;
        on.surface.since = ++order;
      } else if (pressedSurface !== on) {
        pressedSurface = null;
      }
      updateKeyboard();
      return;
    }
    case "key": {
      const holder = keyboardHolder();
      if (!holder) return;
      if (focusedInput && surfaceOf(focusedInput) === holder && focusedInput.input!.key(event)) {
        return;
      }
      const used = holder.surface!.onKey?.(event) === true;
      if (!used && event.pressed && event.key === "Escape") holder.surface!.onEscape?.();
      return;
    }
    case "ran": {
      const done = running.get(event.id);
      running.delete(event.id);
      done?.(event.code);
      return;
    }
    case "style":
      setStyleValue(event.style ?? {});
      return;
    case "settings":
      for (const [name, values] of Object.entries(event.settings ?? {})) {
        settingStores.get(name)?.(values);
      }
      return;
    case "outputs":
      setOutputList(read(native.outputs(), []));
      return;
    case "workspaces":
      setWorkspaceMap(event.outputs ?? {});
      return;
    case "current-output":
      setCurrentValue(event.output ?? null);
      return;
    case "fullscreen":
      setFullscreenMap(event.outputs ?? {});
      return;
    case "overview":
      setOverviewValue(Boolean(event.open));
      return;
    case "windows":
      setWindowList({ list: event.windows ?? [] });
      return;
    case "shell-action":
      for (const fn of [...(actionListeners.get(event.name) ?? [])]) {
        fn({ output: event.output ?? null, args: event.args ?? [] });
      }
      return;
    case "data-ready":
      // The data program started.
      for (const fn of [...(dataListeners.get("ready") ?? [])]) fn({});
      return;
    default: {
      const { type, ...rest } = event;
      if (DATA_EVENTS.has(type)) {
        for (const fn of [...(dataListeners.get(type) ?? [])]) fn(rest);
      } else {
        dataStore(type)[1](rest);
      }
    }
  }
}
