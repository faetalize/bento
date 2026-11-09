// bento's widgets: nodes in bento's scene, which bento lays out, animates and draws.
//
// A node takes props: layout (direction, gap, padding, width, height, grow, align, justify,
// position, top/left/right/bottom, display), look (fill, material, tint, radius, border, opacity,
// clip, color), text (text, size, weight, family, textAlign, tabular) and motion (animate, enter,
// anchor). Changing them later moves the node there with springs. Handlers: onClick, onHover,
// onScroll.

const native = globalThis.__bento;

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
      if (key === "onClick" || key === "onHover" || key === "onScroll") {
        own[key.slice(2).toLowerCase()] = value;
        events = true;
      } else {
        rest[key] = value;
      }
    }
    if (events) {
      rest.events = Object.keys(own).filter((event) => own[event]);
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

export function text(value, props = {}) {
  return new Node("text", { ...props, text: String(value) });
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

// Listens to bento's events, like "workspaces" or "outputs".
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
    case "hover":
    case "scroll":
      handlers.get(event.id)?.[event.type]?.(event);
      return;
    default:
      for (const fn of listeners.get(event.type) ?? []) fn(event);
  }
}
