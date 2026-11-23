// bento's own widgets: buttons, sliders, switches, and islands of glass that open into panels.
// Every other stock module builds from them, so a `modules/ui.tsx` of one's own restyles them all.

import {
  Surface, after, css, paced, settling, type Edge, type JSX, type KeyEvent, type Offset,
  type TextProps,
} from "bento";
import { createEffect, createSignal, For, Show } from "solid-js";

/** What's lit, like the current workspace or a slider's level: the accent, see-through enough
 * to keep the glass under it. */
export const ACTIVE = "accent/0.6";
/** How tall an island is, closed. */
export const ISLAND = 36;
/** The room on either side of an icon that's a button, inside its hover. */
export const ICON_ROOM = 6;

// ------------------------------------------------------------------------------------------------
// What's open over the desktop: one panel, launcher or editor at a time, by a key of its own, and
// what's open over it that came out of it, like a menu over the rest of the tray.
// ------------------------------------------------------------------------------------------------

const [current, setCurrent] = createSignal<string | null>(null);
const [under, setUnder] = createSignal<string | null>(null);
// What's open as it is now: Solid shows what's set once the event is done, and a press outside
// two of them closes both in one event.
let now: { current: string | null; under: string | null } = { current: null, under: null };
const set = (top: string | null, below: string | null = null) => {
  now = { current: top, under: below };
  setCurrent(top);
  setUnder(below);
};
// What a press outside it closed just now: the press on its own button that closed it doesn't
// open it again as it ends.
let dismissed: { key: string; at: number } | null = null;

export const popup = {
  /** What's open on top, by its key. */
  current,
  /** Whether `key` is open, on top or under what came out of it. */
  is: (key: string) => current() === key || under() === key,
  /** Opens `key` in place of what's open, or with `over`, over it, as a menu opens over the
   * flyout it's asked for in, which stays until it closes. */
  open: (key: string, options?: { over?: boolean }) => {
    const below = options?.over && now.current !== key ? now.current : null;
    set(key, below);
  },
  /** Closes `key`, back to what it's over, or whatever's open. */
  close: (key?: string) => {
    if (key === undefined || now.under === key) set(null);
    else if (now.current === key) set(now.under);
  },
  /** Closes `key` for a press outside it, back to what it's over. What's under something else
   * closes too only when the press was outside that as well, which every surface hears of
   * before it's settled. */
  dismiss: (key: string) => {
    if (now.current === key) {
      dismissed = { key, at: Date.now() };
      set(now.under);
    } else if (now.under === key) {
      queueMicrotask(() => {
        // What was over it closed for the same press, so the press was outside both.
        if (now.current !== key) return;
        dismissed = { key, at: Date.now() };
        set(null);
      });
    }
  },
  toggle: (key: string) => {
    const just = dismissed?.key === key && Date.now() - dismissed.at < 500;
    dismissed = null;
    if (just) return;
    set(now.current === key || now.under === key ? null : key);
  },
};

// ------------------------------------------------------------------------------------------------
// Buttons.
// ------------------------------------------------------------------------------------------------

const s = css({
  button: {
    flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8,
    borderRadius: 14, height: 28, cursor: "pointer", flexShrink: 0,
    ":hover": { background: "fg/0.12" },
  },
  active: { background: ACTIVE, color: "#ffffff", ":hover": { background: ACTIVE } },
  label: { fontWeight: 600 },
});

export interface ButtonProps {
  icon?: string;
  label?: string | number;
  /** Lit with the accent. */
  active?: boolean;
  size?: number;
  onClick?: (event: any) => void;
  onRightClick?: (event: any) => void;
  style?: any;
}

/** An icon, a label, or both, in a pill that lights up under the pointer. */
export function Button(props: ButtonProps) {
  return (
    <box
      style={[
        s.button,
        { paddingLeft: props.label !== undefined ? 12 : ICON_ROOM },
        { paddingRight: props.label !== undefined ? 12 : ICON_ROOM },
        props.active && s.active,
        props.style,
      ]}
      onClick={(event) => props.onClick?.(event)}
      onRightClick={props.onRightClick}
    >
      <Show when={props.icon}>
        <image icon={props.icon} size={props.size ?? 16} />
      </Show>
      <Show when={props.label !== undefined}>
        <text style={s.label}>{props.label}</text>
      </Show>
    </box>
  );
}

// ------------------------------------------------------------------------------------------------
// Sliders and switches.
// ------------------------------------------------------------------------------------------------

const SLIDER_HEIGHT = 28;

export interface SliderProps {
  /** From 0 to 1. */
  value: number;
  width?: number;
  icon?: string;
  onChange?: (value: number) => void;
  /** A click on the icon at its start. */
  onIcon?: () => void;
}

/** A track that fills up to its value, which pressing, dragging and scrolling set. */
export function Slider(props: SliderProps) {
  const width = () => props.width ?? 240;
  // What it's set to here, until the value from outside says the same.
  const [set, setSet] = createSignal<number | null>(null);
  createEffect(
    () => props.value,
    () => {
      setSet(null);
    },
  );
  const value = () => set() ?? props.value;
  const put = (next: number) => {
    const clamped = Math.min(Math.max(next, 0), 1);
    if (clamped === value()) return;
    setSet(clamped);
    props.onChange?.(clamped);
  };
  const at = (event: any) => event.frame?.width > 0 && put(event.offsetX / event.frame.width);
  // The level is the track's width, slid out from the left by as much as it isn't filled, so
  // that it goes wherever the track goes, and only its slide is quick, to keep up with a drag.
  const filled = () => Math.max(SLIDER_HEIGHT, value() * width());
  return (
    <box
      style={{
        width: width(), height: SLIDER_HEIGHT, borderRadius: SLIDER_HEIGHT / 2,
        background: "fg/0.12", overflow: "hidden", cursor: "pointer", flexShrink: 0,
      }}
      onPress={(event) => {
        if (props.onIcon && event.offsetX < SLIDER_HEIGHT) props.onIcon();
        else at(event);
      }}
      onDrag={at}
      onScroll={({ dx, dy }) => put(value() - Math.sign(dy || dx) * 0.05)}
    >
      <box
        style={{
          position: "absolute", top: 0, left: 0, height: SLIDER_HEIGHT, width: width(),
          borderRadius: SLIDER_HEIGHT / 2, background: ACTIVE,
          translate: [filled() - width(), 0], transition: { translate: 0.08 },
        }}
      />
      <Show when={props.icon}>
        <image
          icon={props.icon}
          style={{ position: "absolute", top: 6, left: 6, color: "#ffffff" }}
        />
      </Show>
    </box>
  );
}

/** A switch: a pill whose knob slides to its end when it's on, lit with the accent. */
export function Toggle(props: { on: boolean; onChange?: (on: boolean) => void }) {
  const [WIDTH, HEIGHT, KNOB] = [40, 22, 16];
  const travel = WIDTH - KNOB - 6;
  return (
    <box
      style={{
        width: WIDTH, height: HEIGHT, borderRadius: HEIGHT / 2, flexShrink: 0, cursor: "pointer",
        background: props.on ? ACTIVE : "fg/0.2",
      }}
      onClick={() => props.onChange?.(!props.on)}
    >
      <box
        style={{
          position: "absolute", top: 3, left: 3, width: KNOB, height: KNOB,
          borderRadius: KNOB / 2, background: "#ffffff", translate: [props.on ? travel : 0, 0],
        }}
      />
    </box>
  );
}

// ------------------------------------------------------------------------------------------------
// Islands.
// ------------------------------------------------------------------------------------------------

export interface IslandProps {
  /** Which of its panels it's open into, by key, or none. */
  open?: string | null;
  /** What it can open into, by key. */
  panels?: Record<string, () => JSX.Element>;
  /** The side of the island that stays put as it grows: its start, its middle or its end, so
   * that what it holds never moves on the screen, only the glass around it does. */
  hold?: "start" | "center" | "end";
  /** The panel's corners. */
  radius?: number;
  /** The room around a panel. */
  pad?: number;
  /** The room at the island's sides, closed. */
  padding?: [number, number] | [number, number, number, number];
  gap?: number;
  onClick?: () => void;
  /** The island's glass, as a ref gets it, to put things beside it. */
  ref?: (node: any) => void;
  style?: any;
  children?: JSX.Element;
}

/** A pill of glass that holds a row of things, and opens into a panel. Opening, the island
 * turns into its panel's size, one piece of glass changing shape, and once it's most of the way
 * there the panel sharpens in; closing, the panel blurs out first and the glass goes back to
 * the island. Opening another of its panels, the glass turns straight from one to the other. */
export function Island(props: IslandProps) {
  const pad = () => props.pad ?? 16;
  // How big each panel is laid out, as it says when that changes.
  const [sizes, setSizes] = createSignal<Record<string, [number, number]>>({});
  // Which panel shows: it comes once the glass is most of the way there.
  const [shown, setShown] = createSignal<string | null>(null);
  let stop: (() => void) | null = null;
  createEffect(
    () => props.open ?? null,
    (open) => {
      stop?.();
      stop = null;
      if (!open) {
        setShown(null);
      } else if (shown() !== null) {
        // From another of its panels: the new one comes as the glass turns.
        setShown(null);
        stop = after(paced(settling() * 0.5), () => setShown(open));
      } else {
        stop = after(paced(settling() * 0.5), () => setShown(open));
      }
    },
  );
  const size = () => {
    const open = props.open;
    return open ? sizes()[open] : undefined;
  };
  const [padY, padX] = [0, 14];
  return (
    <box
      ref={(node) => props.ref?.(node)}
      style={[
        {
          material: "glass", flexDirection: "row", alignItems: "start", overflow: "hidden",
          justifyContent: props.hold ?? "start",
          padding: props.padding ?? [padY, padX],
        },
        size()
          ? {
            width: size()![0] + 2 * pad(), height: size()![1] + 2 * pad(),
            borderRadius: props.radius ?? 24,
          }
          : { height: ISLAND, borderRadius: ISLAND / 2 },
        props.style,
      ]}
      onClick={props.onClick}
    >
      <box
        style={{
          flexDirection: "row", height: ISLAND, alignItems: "center", gap: props.gap ?? 6,
          flexShrink: 0,
        }}
      >
        {props.children}
      </box>
      <For each={Object.keys(props.panels ?? {})}>
        {(key) => (
          <PanelHolder
            open={props.open === key}
            shown={shown() === key}
            pad={pad()}
            hold={props.hold ?? "start"}
            onSize={(width, height) => setSizes((all) => ({ ...all, [key]: [width, height] }))}
          >
            {props.panels![key]()}
          </PanelHolder>
        )}
      </For>
    </box>
  );
}

/** Where a panel is laid over its island, out of its flow. It's made as its island opens into
 * it, laid out while hidden so the island knows how big to get, and goes once it has blurred
 * out. Presses on it that nothing in it takes stay in it. */
function PanelHolder(props: {
  open: boolean;
  shown: boolean;
  pad: number;
  hold: "start" | "center" | "end";
  onSize: (width: number, height: number) => void;
  children: JSX.Element;
}) {
  let holder: any;
  // Hidden only once it has blurred out.
  const [visible, setVisible] = createSignal(false);
  // Which time it opened: opened again, even while it blurs out, it's made afresh.
  const [made, setMade] = createSignal<number | null>(null);
  let opened = 0;
  createEffect(
    () => props.open,
    (open) => {
      if (open) setMade(++opened);
    },
  );
  let stop: (() => void) | null = null;
  createEffect(
    () => props.shown,
    (shown) => {
      stop?.();
      stop = null;
      if (shown) {
        setVisible(true);
        holder?.jump({ opacity: 0, blur: 6 });
      } else {
        stop = after(paced(120), () => {
          setVisible(false);
          if (!props.open) setMade(null);
        });
      }
    },
  );
  return (
    <box
      ref={holder}
      style={{
        position: "absolute", top: props.pad, left: 0, right: 0,
        paddingLeft: props.pad, paddingRight: props.pad,
        flexDirection: "row", justifyContent: props.hold,
        visibility: visible() ? "visible" : "hidden",
        opacity: props.shown ? 1 : 0, blur: props.shown ? 0 : 6,
        transition: props.shown ? undefined : { opacity: 0.1, blur: 0.1 },
      }}
    >
      <Show when={made()} keyed>
        <box
          style={{ flexShrink: 0, flexDirection: "column" }}
          onPress={() => {}}
          onLayout={(rect) => props.onSize(rect.width, rect.height)}
        >
          {props.children}
        </box>
      </Show>
    </box>
  );
}

// ------------------------------------------------------------------------------------------------
// Lists.
// ------------------------------------------------------------------------------------------------

// ------------------------------------------------------------------------------------------------
// Scrolling.
// ------------------------------------------------------------------------------------------------

/** The scrollbar's thumb at its smallest, and how long it stays after scrolling, in ms. */
const THUMB_LEAST = 24;
const THUMB_LINGER = 1000;

/** A box whose content scrolls, `style` its size and `inner` how it lays its content out, with a
 * thin bar along its right side that shows where it is and how much there is, as libadwaita's
 * overlay scrollbars do: it shows while the pointer is near or it scrolls, widens under the
 * pointer, drags, and a press in its track goes a page up or down. */
export function Scroller(props: {
  style?: any;
  inner?: any;
  /** A span of what it holds, its top and bottom from the top of it, that it scrolls into view
   * whenever it's another, as for what's picked with the keys. */
  reveal?: [number, number] | null;
  children?: JSX.Element;
}) {
  const [scroll, setScroll] = createSignal({ top: 0, reach: 0, height: 0 });
  const [near, setNear] = createSignal(false);
  const [moving, setMoving] = createSignal(false);
  const [over, setOver] = createSignal(false);
  const [held, setHeld] = createSignal(false);
  let box: any;
  let stop: (() => void) | null = null;
  // Where a drag of the thumb started: the pointer and how far it was scrolled.
  let grab: { y: number; top: number } | null = null;
  const thumb = () => {
    const { top, reach, height } = scroll();
    if (reach <= 0 || height <= 0) return null;
    const size = Math.min(height, Math.max(THUMB_LEAST, (height * height) / (height + reach)));
    return { size, at: ((height - size) * top) / reach };
  };
  const shown = () => near() || moving() || over() || held();
  const wide = () => over() || held();
  createEffect(
    () => props.reveal,
    (span) => {
      if (!span) return;
      const { top, height } = scroll();
      if (span[0] < top) box?.scrollTo(span[0]);
      else if (span[1] > top + height) box?.scrollTo(span[1] - height);
    },
  );
  return (
    <box style={[{ flexDirection: "column" }, props.style]} onNear={({ inside }) => setNear(inside)}>
      <box
        ref={box}
        style={[{ overflow: "scroll", flexGrow: 1, flexBasis: 0, minHeight: 0 }, props.inner]}
        onScrolled={(event) => {
          setScroll(event);
          setMoving(true);
          stop?.();
          stop = after(THUMB_LINGER, () => setMoving(false));
        }}
      >
        {props.children}
      </box>
      <Show when={thumb()}>
        {/* The track, which pages towards where it's pressed. */}
        <box
          style={{
            position: "absolute", top: 0, bottom: 0, right: 0, width: 12,
            opacity: shown() ? 1 : 0, transition: { opacity: 0.2 },
          }}
          onHover={({ inside }) => setOver(inside)}
          onClick={(event) => {
            const { top, height } = scroll();
            box?.scrollTo(event.offsetY < thumb()!.at ? top - height : top + height);
          }}
        >
          <box
            style={{
              position: "absolute", top: thumb()!.at, right: 2, width: wide() ? 8 : 4,
              height: thumb()!.size, borderRadius: 4,
              background: wide() ? "fg/0.5" : "fg/0.3",
              // It keeps up with what scrolls.
              animate: false, cursor: "default",
            }}
            onPress={(event) => {
              grab = { y: event.y, top: scroll().top };
              setHeld(true);
            }}
            onDrag={(event) => {
              const now = thumb();
              if (!grab || !now) return;
              const { reach, height } = scroll();
              box?.scrollTo(grab.top + ((event.y - grab.y) * reach) / Math.max(1, height - now.size));
            }}
            onRelease={() => {
              grab = null;
              setHeld(false);
            }}
          />
        </box>
      </Show>
    </box>
  );
}

/** A faint line between groups of things in a row, `length` long, 16 unless it says. */
export function Divider(props: { length?: number; style?: any }) {
  return (
    <box
      style={[
        { width: 1, height: props.length ?? 16, background: "fg/0.15", flexShrink: 0 },
        props.style,
      ]}
    />
  );
}

/** A section's title. */
export function Heading(props: { children: TextProps["children"] }) {
  return <text style={{ fontSize: 12, fontWeight: 700, color: "fg/0.6" }}>{props.children}</text>;
}

/** A dim line, for a list with nothing in it. */
export function Note(props: { children: TextProps["children"] }) {
  return (
    <box style={{ padding: [8, 12] }}>
      <text style={{ color: "fg/0.6" }}>{props.children}</text>
    </box>
  );
}

export interface RowProps {
  icon?: string | null;
  title: string;
  subtitle?: string;
  /** Lit, as what's in use. */
  lit?: boolean;
  onClick?: () => void;
  /** What's at its end. */
  children?: JSX.Element;
}

/** A row of a list: an icon, a title and a word under it, and what's at its end. */
export function Row(props: RowProps) {
  return (
    <box
      style={{
        flexDirection: "row", alignItems: "center", gap: 12, minHeight: 40,
        padding: [6, 6, 6, 12], borderRadius: 12,
        background: props.lit ? "fg/0.1" : undefined,
        cursor: props.onClick ? "pointer" : undefined,
        ":hover": props.onClick ? { background: "fg/0.14" } : undefined,
      }}
      onClick={props.onClick ? () => props.onClick!() : undefined}
      enter={{ opacity: 0 }}
      exit={{ opacity: 0 }}
    >
      <Show when={props.icon}>
        <image icon={props.icon!} style={{ color: props.lit ? "accent" : "fg" }} />
      </Show>
      <box style={{ flexDirection: "column", flexGrow: 1, flexShrink: 1, minWidth: 0 }}>
        <text style={{ fontWeight: props.lit ? 700 : 400, lineClamp: 1 }}>{props.title}</text>
        <Show when={props.subtitle}>
          <text style={{ fontSize: 12, color: "fg/0.6", lineClamp: 1 }}>{props.subtitle}</text>
        </Show>
      </box>
      {props.children}
    </box>
  );
}

/** Text without its markup, at most `length` long, as notifications and the clipboard give it. */
export function plain(markup: unknown, length = 180) {
  const flat = String(markup ?? "")
    .replace(/<[^>]*>/g, "")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
  return flat.length > length ? `${flat.slice(0, length - 1).trimEnd()}…` : flat;
}

// ------------------------------------------------------------------------------------------------
// Coming out of nothing.
// ------------------------------------------------------------------------------------------------

/** The poses of what grows out of a point in the middle of its place, its corners easing from
 * round, and shrinks back into one: `<box {...grows(18)}>`. What it holds that's to grow with it
 * takes `growsWith`. */
export function grows(radius: number, size = 44) {
  return {
    enter: { drawSize: [0, 0] as [number, number], borderRadius: size / 2 },
    exit: { drawSize: [0, 0] as [number, number], borderRadius: size / 2 },
  };
}

/** The poses of what grows with what holds it, around its own middle, sharpening in. */
export const growsWith = {
  enter: { opacity: 0, blur: 4, scale: 0 },
  exit: { opacity: 0, blur: 4, scale: 0 },
};

// ------------------------------------------------------------------------------------------------
// Flyouts.
// ------------------------------------------------------------------------------------------------

export interface FlyoutProps {
  output: string;
  /** What `popup` has it as, which opens and closes it, one of what's open over the desktop. */
  popup?: string;
  /** Whether it's open, for one that isn't one of `popup`'s, like previews on hover. */
  open?: boolean;
  /** What it comes out of, on its `side`, lined up with its `align`. */
  from?: any;
  side?: Edge;
  align?: "start" | "center" | "end";
  /** Without `from`, the edges it's held to. */
  anchor?: Edge[];
  /** How far from what it comes out of, or from the output's edges. */
  gap?: number;
  /** Whether it takes the keys while it's open. */
  keyboard?: boolean;
  radius?: number;
  padding?: number;
  /** Whether a press outside it closes it. */
  light?: boolean;
  /** Without `from`, a point on the output it comes out of, as a menu at the pointer does. */
  at?: { x: number; y: number };
  /** The keys it has while it takes them, before Escape closes it: `true` when one was used. */
  onKey?: (event: KeyEvent) => boolean | void;
  /** The layer it's in, and its place there, the top layer over the shell's bars unless it
   * says, as a menu goes over the shell in the overlay layer. */
  layer?: "background" | "bottom" | "top" | "overlay";
  z?: number;
  children?: JSX.Element;
}

/** A panel of glass that comes out of a button, as quick settings come out of a taskbar, and
 * goes back into it: it slides out of its edge facing the button, as if from behind it, its glass
 * melting out of the bar's while it's close. Held to the output's edges instead, it fades and
 * slides in. It's made afresh each time it opens, and Escape or a press elsewhere closes it. */
export function Flyout(props: FlyoutProps) {
  const open = () => props.open ?? (props.popup !== undefined && popup.is(props.popup));
  const close = () => props.popup !== undefined && popup.close(props.popup);
  const side = () => props.side ?? "top";
  const attached = () => !!(props.from || props.at);
  const across = () => side() === "top" || side() === "bottom";
  // The edge facing what it comes out of, which it slides out of.
  const edge = () =>
    ({ top: "bottom", bottom: "top", left: "right", right: "left" } as const)[side()];
  // Towards what it comes out of, by its whole size.
  const behind = (): [Offset, Offset] => {
    const away: Record<Edge, [Offset, Offset]> = {
      top: [0, "100%"], bottom: [0, "-100%"], left: ["100%", 0], right: ["-100%", 0],
    };
    return away[side()];
  };
  // Its glass is cut down to that edge, and what it holds slides with the glass's far edge.
  const glassPose = () =>
    attached()
      ? { drawSize: (across() ? ["100%", 0] : [0, "100%"]) as [Offset, Offset] }
      : { opacity: 0, translate: slide() };
  const contentPose = () => (attached() ? { translate: behind() } : {});
  // Held to the output's edges, it slides in from the side it comes out of.
  const slide = (): [number, number] =>
    side() === "top" ? [0, 12] : side() === "bottom" ? [0, -12] : side() === "left" ? [12, 0] : [-12, 0];
  // Made as it opens, so its poses are plain values there as it's made, before its effects run.
  const Panel = () => {
    const glass = glassPose();
    const content = contentPose();
    return (
      <box
        style={{
          material: "glass", borderRadius: props.radius ?? 18, padding: props.padding ?? 16,
          overflow: "hidden", flexDirection: "column", drawFrom: edge(),
        }}
        enter={glass}
        exit={glass}
      >
        <box style={{ flexDirection: "column" }} enter={content} exit={content}>
          {props.children}
        </box>
      </box>
    );
  };
  return (
    <Surface
      output={props.output}
      layer={props.layer ?? "top"}
      z={props.z ?? 2}
      attach={props.from || props.at ? {
        to: props.from ?? props.at!, side: side(), align: props.align ?? "center",
        gap: props.gap ?? 12,
      } : undefined}
      anchor={props.from || props.at ? undefined : props.anchor}
      keyboard={open() && props.keyboard ? "exclusive" : "none"}
      onKey={(event) => props.onKey?.(event)}
      onEscape={close}
      onOutside={() => props.light !== false && props.popup !== undefined && popup.dismiss(props.popup)}
      style={{ margin: props.gap ?? 12 }}
    >
      <Show when={open()}>
        <Panel />
      </Show>
    </Surface>
  );
}
