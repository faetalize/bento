// bento's widgets: nodes in bento's scene, which bento lays out, animates and draws.
//
// A node takes props: layout (direction, gap, padding, width, height, grow, align, justify,
// position, top/left/right/bottom, display), look (fill, material, tint, radius, border,
// opacity, scale, blur, clip, color, cursor, hidden), text (text, size, weight, family,
// textAlign, tabular) and motion.
//
// The shell is one layer, drawn in the order its nodes are added: above the windows, fullscreen
// ones too, and under other programs' overlay surfaces.
//
// bento moves a node to where its layout and look say with critically damped springs, which
// keep their speed when sent somewhere else on the way. What a script moves besides the layout:
// - `offset: [dx, dy]` moves it from its place;
// - `drawSize: [w, h]` draws it at that size around the middle of its place, or at its place's
//   size with `null`;
// - `opacity`, `radius`, `scale`, `blur`.
// `jump: { ... }` puts any of those somewhere first, to move from there. `response` is how long
// its moves take, in seconds, and `transition: { frame, opacity, radius, scale, blur, offset,
// size }` each kind's. `animate: false` puts it at its place at once. `hidden: true` leaves it
// out at once, neither drawn nor hit, and so does `display: "none"`, which leaves it out of the
// layout too; shown again, it comes in at its place.
//
// How things come, go and turn into each other is built from those here: `grow` and `shrink`,
// `travel` and `home`, and panels that `expand` out of their islands.
//
// Handlers, each with its event:
// - onClick, onMiddleClick, onRightClick: pressed and let go on it, with that button, or a touch
//   or a tablet pen as the left one.
// - onPress: pressed, with `button` 1, 2 or 3 for left, middle and right.
// - onHover: the pointer came onto it or left it, with `inside`.
// - onScroll: scrolled over it, by `dx` and `dy`.
// - onSettle: its motion settled.
// - onNear: the pointer came into its place or left it, with `inside`, whatever else is there,
//   in the shell or under it. It takes nothing from the pointer: a zone to know it's near.
// - onDrag: the pointer moved while a press on it is held, to `x` and `y` on the output.
//
// Shapes and what has a click, press, hover or scroll handler take the pointer from what's below
// them; the rest lets it through. `cursor` is how the pointer looks over a node: "default", "pointer", "text" or
// "grab".

const native = globalThis.__bento;

const HANDLERS = [
  "onClick", "onMiddleClick", "onRightClick", "onPress", "onHover", "onScroll", "onSettle",
  "onNear", "onDrag",
];
const CLICKS = { 1: "onClick", 2: "onMiddleClick", 3: "onRightClick" };

const handlers = new Map();
const timers = new Map();
const listeners = new Map();

export class Node {
  constructor(kind, props = {}, children = []) {
    this.id = native.create(kind, "{}");
    handlers.set(this.id, {});
    this.set(props);
    for (const child of children) {
      if (child) this.add(child);
    }
  }

  static wrap(id) {
    const node = Object.create(Node.prototype);
    node.id = id;
    if (!handlers.has(id)) handlers.set(id, {});
    return node;
  }

  set(props) {
    const own = handlers.get(this.id);
    const rest = {};
    let events = false;
    for (const [key, value] of Object.entries(props)) {
      if (HANDLERS.includes(key)) {
        own[key] = value;
        events = true;
      } else {
        rest[key] = value;
      }
    }
    if (events) {
      const has = (keys) => keys.some((key) => own[key]);
      rest.events = [
        has(["onClick", "onMiddleClick", "onRightClick", "onPress"]) && "click",
        has(["onHover"]) && "hover",
        has(["onSettle"]) && "settle",
        has(["onScroll"]) && "scroll",
        has(["onNear"]) && "near",
        has(["onDrag"]) && "drag",
      ].filter(Boolean);
    }
    native.set(this.id, JSON.stringify(rest));
    return this;
  }

  add(child, index = -1) {
    native.append(this.id, child.id, index);
    return this;
  }

  remove() {
    native.remove(this.id);
    handlers.delete(this.id);
  }
}

export function box(props = {}, children = []) {
  return new Node("box", props, children);
}

// A text field. bento keeps its text, caret and selection, and what the keys and clicks do to
// them; everything about how it looks is here, in plain defaults that `look` overrides:
// `{ caret, selection, placeholder }`, each a node's props. Its own props, like `size` or
// `color`, are its text's. `onChange(text)` hears every change, and `onSubmit(text)` Enter.
// `focus()` gives it the keys, while the shell has the keyboard.
export function input({
  placeholder = "",
  value = "",
  onChange,
  onSubmit,
  look = {},
  ...props
} = {}) {
  const field = new Node("input", { text: value, ...props });
  const hint = text(placeholder, {
    position: "absolute", top: 0, left: 0, color: "fg/0.45", ...props, ...look.placeholder,
  });
  const caret = box({
    position: "absolute", top: 0, left: 0, width: 1.5, height: props.size ?? 14,
    fill: "fg", opacity: 0, radius: 1, ...look.caret,
  });
  const marks = [];
  // It takes the width of the row it's in, so its placeholder isn't wrapped to the width of
  // what's typed, and a press anywhere along it lands in it.
  const wrap = box({ grow: 1, shrink: 1, minWidth: 0, onPress: (event) => {
    focus(widget);
    show(native.edit(field.id, JSON.stringify({ point: [event.x, event.y] })));
  } }, [field, hint, caret]);

  let blink = null;
  let current = value;
  let lastTyped = 0;
  const show = (json) => {
    if (!json) return;
    const state = JSON.parse(json);
    const [x, y, , h] = state.caretRect;
    // Moved, the caret glides there at bento's pace; typing, it glides as fast as the typing
    // goes, so it keeps up with every letter.
    const typed = state.text !== current;
    let response = null;
    if (typed) {
      const now = Date.now();
      response = Math.min(Math.max((now - lastTyped) / 2000, 0.04), style().response ?? 0.42);
      lastTyped = now;
    }
    caret.set({ left: x, top: y, height: h, opacity: widget.focused ? 1 : 0, response });
    hint.set({ opacity: state.text ? 0 : 1 });
    marks.splice(0).forEach((mark) => mark.remove());
    for (const [sx, sy, sw, sh] of state.selection) {
      const mark = box({
        position: "absolute", left: sx, top: sy, width: sw, height: sh,
        fill: "accent/0.35", animate: false, ...look.selection,
      });
      wrap.add(mark, 0);
      marks.push(mark);
    }
    if (state.text !== current) {
      current = state.text;
      onChange?.(current);
    }
    // The caret stays lit while typing, and blinks once the typing stops.
    if (blink) cancel(blink);
    if (widget.focused) {
      let lit = true;
      const tick = () => {
        lit = !lit;
        caret.set({ opacity: lit ? 1 : 0 });
        blink = after(530, tick);
      };
      blink = after(530, tick);
    }
    return state;
  };

  const widget = Object.assign(wrap, {
    focused: false,
    setText(value) {
      field.set({ text: value });
      show(native.edit(field.id, "{}"));
    },
    focus() {
      focus(widget);
    },
    // Takes a key while it has the focus, and says whether it used it.
    key(event) {
      if (!event.pressed) return false;
      if (event.key === "Return" || event.key === "KP_Enter") {
        onSubmit?.(current);
        return true;
      }
      const state = show(native.edit(field.id, JSON.stringify(event)));
      return Boolean(state?.changed);
    },
    blurred() {
      if (blink) cancel(blink);
      blink = null;
      caret.set({ opacity: 0 });
    },
  });
  // What it says now.
  Object.defineProperty(widget, "text", { get: () => current });
  show(native.edit(field.id, "{}"));
  return widget;
}

// The text field that gets the keys first, or none.
let focused = null;

export function focus(field) {
  if (focused === field) return;
  if (focused) {
    focused.focused = false;
    focused.blurred();
  }
  focused = field;
  if (field) {
    field.focused = true;
    field.setText(field.text);
  }
}

// An icon from the icon theme by name, or an image file by its path, `size` pixels square
// unless `width` and `height` say. A symbolic icon, whose name ends in "-symbolic", is drawn in
// the node's `color`.
export function image(source, props = {}) {
  const from = String(source).startsWith("/") && !props.icon ? { src: source } : { icon: source };
  // An icon keeps its size when a row is short of room; the text beside it gives way.
  return new Node("image", { size: 16, shrink: 0, ...from, ...props });
}

export function text(value, props = {}) {
  return new Node("text", { ...props, text: String(value) });
}

// Runs a bento action, as a bind would, like `action("focus-workspace", [], { workspace: id })`
// or `action("spawn", ["foot"])`. `properties`, `window` and `workspace` are optional.
export function action(name, args = [], { properties = {}, window, workspace } = {}) {
  native.action(JSON.stringify({ name, args, properties, window, workspace }));
}

// The apps that show in launchers, sorted by name: `{ id, name, generic, comment, keywords,
// icon }` each.
export function apps() {
  return JSON.parse(native.apps());
}

// Loads the icon `name` at `size` ahead of its first use, so that it's there in the frame it's
// first shown in. An icon that isn't loaded yet shows a few frames late.
export function preload(name, size = 16) {
  native.preload(String(name), size);
}

// Starts the app with desktop entry id `id`.
export function launch(id) {
  return native.launch(id);
}

// Tells the shell's data program to do something: `data("volume", { value: 0.5 })`, "mute"
// `{ value }`, "dismiss", "expire" and "invoke" a notification `{ id, key }`, "activate",
// "secondary" and "context" a tray item `{ id }`, "suspend", "reboot" and "power-off". For
// sound devices, with `kind` "output" or "input" and the device's `name`: "device" makes it the
// default, "device-volume" `{ value }` and "device-mute" `{ value }` set it. "brightness" `{ id,
// value }` sets a display's, and "brightness-refresh" reads them again. "theme" `{ dark }` sets
// the light or dark style. "weather" `{ latitude, longitude, fahrenheit }` fetches the weather
// there from now on, and without coordinates stops.
export function data(cmd, fields = {}) {
  native.data(JSON.stringify({ cmd, ...fields }));
}

// Runs `command` with `sh -c`, and calls `done` with its exit code once it's over.
const running = new Map();
let nextRun = 0;
export function run(command, done) {
  const id = nextRun++;
  if (done) running.set(id, done);
  data("run", { id, command });
}

// Shows a notification, with an icon by its name or path.
// Shows a notification. A `transient` one, like saying which sound output plays now, is only a
// toast, and isn't kept in the control center.
export function notify(summary, body = "", icon = "", { transient = false } = {}) {
  data("notify", { summary, body, icon, transient });
}

// The last event of a kind that came, like `last("audio-devices")`, for what's built after it.
const latest = new Map();
export function last(type) {
  return latest.get(type);
}

// What the config sets for the shell: `{ location: { name, latitude, longitude } or null,
// fahrenheit, screenshot and screenRecord: { command, notification, body } or null }`. A
// "config" event tells when it changes.
export function config() {
  return style().config ?? {};
}
let lastConfig = JSON.stringify(config());

// What scripts keep between runs, by key: `stored("pinned", [])` reads, with what to give when
// there's nothing, and `store("pinned", names)` writes.
export function stored(key, fallback = null) {
  return JSON.parse(native.stored())[key] ?? fallback;
}

export function store(key, value) {
  native.store(JSON.stringify({ ...JSON.parse(native.stored()), [key]: value }));
}

// What's lit, like the current workspace, today or a slider's level: the accent, see-through
// enough to keep the glass under it.
export const ACTIVE = "accent/0.6";

// What's open over the desktop, a panel or the launcher: one at a time, with the keyboard while
// it is, closed by Escape or a press elsewhere. Each is `{ close() }`: `opened(it)` when it
// opens closes the others, and `closed(it)` when it closes gives the keyboard back.
const open = new Set();

export function opened(panel) {
  for (const other of [...open]) {
    if (other !== panel) other.close();
  }
  open.add(panel);
  native.keyboard(true);
}

export function closed(panel) {
  open.delete(panel);
  if (open.size === 0) {
    focus(null);
    native.keyboard(false);
  }
}

export function closeAll() {
  for (const panel of [...open]) panel.close();
}

// How tall an island is, closed.
export const ISLAND = 36;

// An island: a pill of glass that holds a row of things, and can open into a panel. What it
// holds is `island.row`'s.
export function island(props = {}, children = []) {
  const { gap = 6, padding = [0, 14], ...rest } = props;
  const row = box({ height: ISLAND, align: "center", gap }, children);
  const pill = box(
    {
      material: "glass", radius: ISLAND / 2, height: ISLAND, padding, align: "start",
      clip: true, ...rest,
    },
    [row],
  );
  pill.row = row;
  return pill;
}

// Opens an island into a panel. `content` is laid over the island, out of its flow, and the
// island grows to its size with `pad` around it; once the glass is most of the way there, the
// content sharpens in at its place. Closing, the content blurs out first and the glass shrinks
// back to an island as wide as what it holds. `onToggle(open)` hears it open and close, as it
// starts. Gives `{ open, setOpen(open), toggle(), close(), fit() }`: `fit()` sizes an open
// island again after its content changed.
//
// An island can open into more than one panel: opening one while another is open turns the glass
// straight from one's size to the other's.
export function expand(pill, content, { radius = 24, pad = 16, hold = "start", onToggle } = {}) {
  const inner = content;
  // Presses on the panel that nothing in it takes stay in it, and it keeps its size in an
  // island narrower than it.
  content.set({ shrink: 0, onClick: () => {} });
  // The content is held to the island's edge that stays put as it grows and shrinks, its start,
  // its middle or its end, so that it never moves on the screen: only the glass around it does.
  const holder = box(
    {
      position: "absolute", top: pad, left: 0, right: 0, padding: [0, pad], hidden: true,
      opacity: 0, justify: hold,
    },
    [content],
  );
  pill.add(holder);
  content = holder;
  let step;
  const later = (ms, fn) => {
    if (step !== undefined) cancel(step);
    step = after(paced(ms), () => {
      step = undefined;
      fn();
    });
  };
  // The content blurs out faster than the glass's edge comes.
  const leave = () => {
    content.set({ opacity: 0, blur: 6, transition: { opacity: 0.1, blur: 0.1 } });
    later(120, () => {
      content.set({ hidden: true, transition: { opacity: null, blur: null } });
    });
  };
  const panel = {
    open: false,
    close: () => panel.setOpen(false),
    toggle: () => panel.setOpen(!panel.open),
    fit() {
      if (!panel.open) return;
      const size = layout(inner);
      if (size) pill.set({ width: size[2] + 2 * pad, height: size[3] + 2 * pad, radius });
    },
    // Closes it for another panel of its island, which the glass turns to without shrinking.
    handOver() {
      if (!panel.open) return;
      panel.open = false;
      leave();
      closed(panel);
      onToggle?.(false);
    },
    setOpen(open) {
      if (panel.open === open) return;
      if (open) {
        for (const other of pill.panels) if (other !== panel) other.handOver();
      }
      panel.open = open;
      if (open) {
        opened(panel);
        panel.fit();
        later(settling() * 0.5, () => {
          content.set({ hidden: false, opacity: 1, blur: 0, jump: { opacity: 0, blur: 6 } });
        });
      } else {
        leave();
        pill.set({ width: "auto", height: ISLAND, radius: ISLAND / 2 });
        closed(panel);
      }
      onToggle?.(open);
    },
  };
  pill.panels = [...(pill.panels ?? []), panel];
  return panel;
}

// Fills `list` like a drawer: it's cut to its own frame, so rows that come show as it opens over
// them, and rows that go stay while it closes over them, and leave once it's closed. It sets the
// rows' `display`.
export function drawer(list, children) {
  children = children.filter(Boolean);
  const before = layout(list)?.[3] ?? 0;
  const old = list.held ?? [];
  // Rows still on their way out from the last change go now.
  for (const row of list.leaving ?? []) row.remove();
  list.leaving = null;
  list.held = children;
  for (const child of children) list.add(child);
  for (const row of old) row.set({ display: "none" });
  list.set({ clip: true, height: "auto" });
  const after = layout(list)?.[3] ?? 0;
  if (after >= before || old.length === 0) {
    for (const row of old) row.remove();
    return;
  }
  // Closing: the old rows stay, and the new ones take their places once it's closed.
  for (const row of old) row.set({ display: "flex" });
  for (const child of children) child.set({ display: "none" });
  list.leaving = old;
  list.set({
    height: after,
    onSettle: () => {
      if (list.leaving !== old) return;
      for (const row of old) row.remove();
      list.leaving = null;
      for (const child of children) child.set({ display: "flex" });
      list.set({ height: "auto" });
    },
  });
}

// Replaces what `node` holds with `children`.
export function refill(node, children) {
  for (const child of node.held ?? []) child.remove();
  node.held = children.filter(Boolean);
  for (const child of node.held) node.add(child);
}

// A button: an icon, a label, or both, in a pill that lights up under the pointer. `active`
// fills it with the accent colour. Its other props are its box's.
export function button({ icon, label, onClick, active = false, size = 16, ...props } = {}) {
  let lit = active;
  const idle = () => (lit ? ACTIVE : props.fill ?? null);
  const ink = () => (lit ? "#ffffff" : "fg");
  const glyph = icon ? image(icon, { size, color: ink() }) : null;
  const words = label !== undefined ? text(label, { weight: 600, color: ink() }) : null;
  const pill = box(
    {
      align: "center", justify: "center", gap: 8, radius: 14, height: 28,
      padding: [0, words ? 12 : 6], cursor: "pointer", ...props,
      fill: idle(),
      onClick: (event) => onClick?.(event),
      onHover: ({ inside }) => pill.set({ fill: inside && !lit ? "fg/0.12" : idle() }),
    },
    [glyph, words],
  );
  // Whether it's lit.
  Object.defineProperty(pill, "active", { get: () => lit });
  // Shows it lit or not, and with another icon or label if they're given.
  pill.show = ({ active, icon, label } = {}) => {
    if (active !== undefined) lit = active;
    pill.set({ fill: idle() });
    glyph?.set({ color: ink(), ...(icon ? { icon } : {}) });
    words?.set({ color: ink(), ...(label !== undefined ? { text: String(label) } : {}) });
  };
  return pill;
}

// A slider: a track `width` wide that fills up to its value, from 0 to 1, with an icon at its
// start. Pressing, dragging and scrolling set it, and `onChange(value)` hears each change;
// `onIcon` hears a click on the icon. `set(value)` shows a value from elsewhere.
export function slider({ value = 0, width = 240, icon, onChange, onIcon } = {}) {
  const HEIGHT = 28;
  let current = value;
  // The level is the track's width, slid out from the left by as much as it isn't filled, so
  // that it goes wherever the track goes, and only its slide is quick, to keep up with a drag.
  const filled = (v) => Math.max(HEIGHT, v * width);
  const fill = box({
    position: "absolute", top: 0, left: 0, height: HEIGHT, width,
    radius: HEIGHT / 2, fill: ACTIVE, offset: [filled(value) - width, 0],
    transition: { offset: 0.08 },
  });
  const glyph = icon
    ? image(icon, { size: 16, color: "#ffffff", position: "absolute", top: 6, left: 6 })
    : null;
  const put = (v, tell) => {
    const next = Math.min(Math.max(v, 0), 1);
    if (next === current) return;
    current = next;
    fill.set({ offset: [filled(next) - width, 0] });
    if (tell) onChange?.(next);
  };
  // Where a press or a drag at `x` on the output puts it.
  const at = ({ x }) => {
    const place = frame(track);
    if (place && place[2] > 0) put((x - place[0]) / place[2], true);
  };
  const track = box(
    {
      width, height: HEIGHT, radius: HEIGHT / 2, fill: "fg/0.12", clip: true, cursor: "pointer",
      onPress: (event) => {
        // The icon is its own button, where it has one.
        const place = frame(track);
        if (onIcon && place && event.x - place[0] < HEIGHT) onIcon();
        else at(event);
      },
      onDrag: at,
      onScroll: ({ dx, dy }) => put(current - Math.sign(dy || dx) * 0.05, true),
    },
    [fill, glyph],
  );
  // Where it is.
  Object.defineProperty(track, "value", { get: () => current });
  // Shows `value`, and another icon if it's given, without telling `onChange`.
  track.show = (value, icon) => {
    if (value !== undefined) put(value, false);
    if (icon) glyph?.set({ icon });
  };
  return track;
}

// A switch: a pill whose knob slides to its end when it's on, lit with the accent. `show(on)`
// shows another state without telling `onChange`.
export function toggle({ on = false, onChange } = {}) {
  const [WIDTH, HEIGHT, KNOB] = [40, 22, 16];
  const travel = WIDTH - KNOB - 6;
  let lit = Boolean(on);
  const knob = box({
    position: "absolute", top: 3, left: 3, width: KNOB, height: KNOB, radius: KNOB / 2,
    fill: "#ffffff", offset: [lit ? travel : 0, 0],
  });
  const set = (value) => {
    lit = value;
    knob.set({ offset: [lit ? travel : 0, 0] });
    track.set({ fill: lit ? ACTIVE : "fg/0.2" });
  };
  const track = box(
    {
      width: WIDTH, height: HEIGHT, radius: HEIGHT / 2, shrink: 0, cursor: "pointer",
      fill: lit ? ACTIVE : "fg/0.2",
      onClick: () => {
        set(!lit);
        onChange?.(lit);
      },
    },
    [knob],
  );
  Object.defineProperty(track, "on", { get: () => lit });
  track.show = (value) => {
    if (Boolean(value) !== lit) set(Boolean(value));
  };
  return track;
}

// Takes the keyboard from the apps, or gives it back. While the shell has it, every key that no
// shortcut takes comes as a "key" event: `{ key, text, pressed, repeat, shift, ctrl, alt, super }`,
// where `key` is its name, like "a", "Return" or "Escape", and `text` what it types.
export function keyboard(take) {
  native.keyboard(Boolean(take));
}

// Sends `node` to where `to` is laid out, as tall as it, its start on `to`'s start and its
// middle on `to`'s middle, as one thing moving to another's place and size. `home(node)` brings
// it back to its own.
export function travel(node, to) {
  const own = layout(node);
  const target = layout(to);
  if (!own || !target || own[3] <= 0 || target[3] <= 0) return;
  // Scaled around its middle, so its start moves by half of what it gains or loses.
  const scale = target[3] / own[3];
  const dx = target[0] - own[0] - (own[2] * (1 - scale)) / 2;
  const dy = target[1] + target[3] / 2 - (own[1] + own[3] / 2);
  node.set({ offset: [dx, dy], scale });
}

export function home(node) {
  node.set({ offset: [0, 0], scale: 1 });
}

// How long `ms` of motion takes with bento's animation settings: slower with a slowdown, and
// none with animations off.
export function paced(ms) {
  const { animations = true, slowdown = 1 } = style();
  return animations ? ms * slowdown : 0;
}

// How long bento's springs take to come most of the way, in milliseconds, at normal speed.
export function settling() {
  return (style().response ?? 0.42) * 1000;
}

// Where a node is drawn now, with its own scale: x, y, width and height.
export function frame(node) {
  return JSON.parse(native.frame(node.id) ?? "null");
}

// Where a node's layout puts it.
export function layout(node) {
  return JSON.parse(native.layout(node.id) ?? "null");
}

// Grows `node` out of a point in the middle of its place to its place, in one motion, its
// corners easing from round to `radius`. Grown again while it shrinks, it turns around from
// wherever it is. `content`, nodes in it, grows with it around its own middle and sharpens in.
export function grow(node, { radius = 0, content = [] } = {}) {
  const place = layout(node);
  if (!place) return;
  const fresh = node.gone !== false && node.gone !== "going";
  node.gone = false;
  node.set({
    hidden: false,
    drawSize: null,
    radius,
    ...(fresh ? { jump: { drawSize: [0, 0], radius: Math.min(place[2], place[3]) / 2 } } : {}),
  });
  for (const child of content) {
    child.set({
      opacity: 1, blur: 0, scale: 1, ...(fresh ? { jump: { opacity: 0, blur: 4, scale: 0 } } : {}),
    });
  }
}

// Shrinks `node` back into a point the way it grew, and leaves it out once it's gone, then
// calls `done`. Shrunk again while it grows, it turns around from wherever it is.
export function shrink(node, { content = [], done } = {}) {
  const place = layout(node);
  if (!place || node.gone) return;
  node.gone = "going";
  for (const child of content) child.set({ opacity: 0, blur: 4, scale: 0 });
  node.set({
    drawSize: [0, 0],
    radius: Math.min(place[2], place[3]) / 2,
    onSettle: () => {
      if (node.gone === "going") {
        node.gone = true;
        node.set({ hidden: true });
        done?.();
      }
    },
  });
}

// bento's style, to line up with: `{ gaps, cornerRadius, animations, slowdown, response }`,
// where `response` is how long bento's springs take to settle, in seconds, as windows move.
// It's sent again as a "style" event whenever it changes.
export function style() {
  return JSON.parse(native.style()) ?? {};
}

// Keeps `edges` of output `name`, like `{ top: 44 }`, for the shell: windows stay out of them.
export function reserve(name, { top = 0, right = 0, bottom = 0, left = 0 } = {}) {
  native.reserve(name, top, right, bottom, left);
}

// The output that has the focus, as the last workspaces said: the one with the focused workspace.
let focusedName = null;

export function focusedOutput() {
  return focusedName;
}

// The outputs, by name.
export function outputs() {
  return JSON.parse(native.outputs());
}

// The node that covers output `name`, to put islands on.
export function root(name) {
  const id = native.root(name);
  return id < 0 ? null : Node.wrap(id);
}

export function after(ms, fn) {
  const id = native.after(ms);
  timers.set(id, fn);
  return id;
}

export function every(ms, fn) {
  const state = {};
  const tick = () => {
    state.id = after(ms, tick);
    fn();
  };
  state.id = after(ms, tick);
  return () => cancel(state.id);
}

export function cancel(id) {
  native.cancel(id);
  timers.delete(id);
}

// Listens to bento's events: "outputs", "workspaces", "style", "fullscreen" `{ outputs: { name:
// whether it shows a fullscreen window } }`, "key" while the shell has the keyboard, "outside",
// a press on the desktop off the shell, to close what's open, and "launcher", the launcher's
// shortcut. The data program's come as events too: "audio" `{ available, volume,
// muted }`, "network" `{ connected, name, wifiStrength, icon }`, "battery" `{ present,
// percentage, charging, icon }`, "tray" `{ items: [{ id, title, icon, menu, attention }] }` and
// "notification" `{ event: "shown" | "replaced" | "closed", id, app, appIcon, summary, body,
// actions, urgency, timeout, image, transient }`, "wifi" `{ available, enabled, joining,
// networks: [{ ssid, strength, secure, known, active }] }` and "wifi-error" `{ ssid, message }`,
// "bluetooth" `{ available, powered, discovering, devices: [{ path, name, icon, paired,
// connected, connecting }] }` and "bluetooth-error" `{ message }`, "power-profile" `{ available,
// active, profiles }`, "media" `{ players: [{ id, name, icon, playing, title, artist, art,
// canNext, canPrevious, canPlay }] }` and "clipboard" `{ available, entries }`.
export function on(type, fn) {
  if (!listeners.has(type)) listeners.set(type, []);
  listeners.get(type).push(fn);
}

export function log(...parts) {
  native.log(parts.map(String).join(" "));
}

// Called by bento with each event, as JSON.
export function __dispatch(json) {
  const event = JSON.parse(json);
  latest.set(event.type, event);
  switch (event.type) {
    case "timer": {
      const fn = timers.get(event.id);
      timers.delete(event.id);
      fn?.();
      return;
    }
    case "click":
      handlers.get(event.id)?.[CLICKS[event.button]]?.(event);
      return;
    case "press":
      handlers.get(event.id)?.onPress?.(event);
      return;
    case "hover":
      handlers.get(event.id)?.onHover?.(event);
      return;
    case "settled":
      handlers.get(event.id)?.onSettle?.(event);
      return;
    case "near":
      handlers.get(event.id)?.onNear?.(event);
      return;
    case "drag":
      handlers.get(event.id)?.onDrag?.(event);
      return;
    case "ran": {
      const done = running.get(event.id);
      running.delete(event.id);
      done?.(event.code);
      return;
    }
    case "style": {
      // What the config sets for the shell rides along with the style.
      const settings = JSON.stringify(event.style?.config ?? {});
      for (const fn of listeners.get("style") ?? []) fn(event);
      if (settings !== lastConfig) {
        lastConfig = settings;
        for (const fn of listeners.get("config") ?? []) fn({ type: "config", config: config() });
      }
      return;
    }
    case "scroll":
      handlers.get(event.id)?.onScroll?.(event);
      return;
    case "workspaces":
      for (const [output, list] of Object.entries(event.outputs ?? {})) {
        if (list.some((ws) => ws.focused)) focusedName = output;
      }
      for (const fn of listeners.get(event.type) ?? []) fn(event);
      return;
    case "key":
      // The focused text field takes what it can use; the rest goes to the listeners, and
      // Escape closes what's open.
      if (focused?.key(event)) return;
      for (const fn of listeners.get(event.type) ?? []) fn(event);
      if (event.pressed && event.key === "Escape") closeAll();
      return;
    case "outside":
      closeAll();
      for (const fn of listeners.get(event.type) ?? []) fn(event);
      return;
    default:
      for (const fn of listeners.get(event.type) ?? []) fn(event);
  }
}
