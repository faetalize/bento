// bento's widgets: nodes in bento's scene, which bento lays out, animates and draws.
//
// A node takes props: layout (direction, gap, padding, width, height, grow, align, justify,
// position, top/left/right/bottom, display), look (fill, material, tint, radius, border,
// opacity, scale, blur, clip, color, cursor), text (text, size, weight, family, textAlign,
// tabular) and motion (animate, enter). Changing them later moves the node there with springs;
// `jump` gives where the look starts from first, like `{ blur: 6 }`.
//
// Handlers, each with its event:
// - onClick, onMiddleClick, onRightClick: pressed and let go on it, with that button, or a touch
//   or a tablet pen as the left one.
// - onPress: pressed, with `button` 1, 2 or 3 for left, middle and right.
// - onHover: the pointer came onto it or left it, with `inside`.
// - onScroll: scrolled over it, by `dx` and `dy`.
//
// Shapes and what has handlers take the pointer from what's below them; the rest lets it
// through. `cursor` is how the pointer looks over a node: "default", "pointer", "text" or
// "grab".

const native = globalThis.__bento;

const HANDLERS = ["onClick", "onMiddleClick", "onRightClick", "onPress", "onHover", "onScroll"];
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
        has(["onScroll"]) && "scroll",
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
  const wrap = box({ onPress: (event) => {
    focus(widget);
    show(native.edit(field.id, JSON.stringify({ point: [event.x, event.y] })));
  } }, [field, hint, caret]);

  let blink = null;
  let current = value;
  const show = (json) => {
    if (!json) return;
    const state = JSON.parse(json);
    const [x, y, , h] = state.caretRect;
    caret.set({ left: x, top: y, height: h, opacity: widget.focused ? 1 : 0 });
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
    get text() {
      return current;
    },
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
  return new Node("image", { size: 16, ...from, ...props });
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

// Starts the app with desktop entry id `id`.
export function launch(id) {
  return native.launch(id);
}

// Takes the keyboard from the apps, or gives it back. While the shell has it, every key that no
// shortcut takes comes as a "key" event: `{ key, text, pressed, repeat, shift, ctrl, alt, super }`,
// where `key` is its name, like "a", "Return" or "Escape", and `text` what it types.
export function keyboard(take) {
  native.keyboard(Boolean(take));
}

// Makes `to` grow out of where `from` is, and `from` travel into where `to` goes, so they read
// as one thing moving to its new place and size. Show one and hide the other as they go.
export function morph(from, to) {
  native.match(from.id, to.id);
}

// bento's style, to line up with: `{ gaps, cornerRadius }`. It's sent again as a "style" event
// whenever it changes.
export function style() {
  return JSON.parse(native.style()) ?? {};
}

// Keeps `edges` of output `name`, like `{ top: 44 }`, for the shell: windows stay out of them.
export function reserve(name, { top = 0, right = 0, bottom = 0, left = 0 } = {}) {
  native.reserve(name, top, right, bottom, left);
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

// Listens to bento's events: "outputs", "workspaces", "style", "key" while the shell has the
// keyboard, "outside", a press anywhere off the shell, to close what's open, and "launcher",
// the launcher's shortcut.
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
    case "scroll":
      handlers.get(event.id)?.onScroll?.(event);
      return;
    case "key":
      // The focused text field takes what it can use; the rest goes to the listeners.
      if (focused?.key(event)) return;
      for (const fn of listeners.get(event.type) ?? []) fn(event);
      return;
    default:
      for (const fn of listeners.get(event.type) ?? []) fn(event);
  }
}
