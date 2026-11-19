// Menus: a list of what can be done with something, at the pointer or out of a button, as a right
// click opens. One is open at a time, among everything `popup` opens, and a submenu opens in its
// place, with a way back, as libadwaita's popover menus do. Any module opens one with
// `openMenu`, whichever shell runs; it shows by itself, with nothing for the shell to place.

import { mountApart, type Edge, type KeyEvent, type OutputName, type SceneNode } from "bento";
import { createSignal, For, Show } from "solid-js";
import { Flyout, popup } from "bento:modules/ui";

/** An item of a menu: what it says, what it does, and a menu of its own, or a line between
 * groups of items. */
export type MenuItem =
  | "separator"
  | {
      label: string;
      icon?: string;
      /** What it does. The menu closes after. */
      run?: () => void;
      /** Shown, but it can't be used now. */
      disabled?: boolean;
      /** A check beside it, for what's on. */
      checked?: boolean;
      /** For what can't be taken back, like closing all of an app's windows. */
      danger?: boolean;
      /** The submenu it opens in the menu's place. */
      items?: MenuItem[];
    };

/** Where a menu opens: at a click or press, which says its output and where on it, or out of a
 * node on an output, on its `side`. */
export type MenuPlace =
  | { output?: OutputName; x: number; y: number }
  | { output: OutputName; node: SceneNode; side?: Edge; align?: "start" | "center" | "end" };

const KEY = "menu";
const WIDTH = 220;

const [shown, setShown] = createSignal<{ place: MenuPlace; items: MenuItem[] } | null>(null);
let mounted = false;

/** Opens a menu of `items` at `place`, in place of any menu or popup that's open. */
export function openMenu(place: MenuPlace, items: MenuItem[]) {
  if (!place.output) return;
  if (!mounted) {
    mounted = true;
    mountApart(() => <Menus />);
  }
  // Made afresh, at its new place, even when one is open.
  setShown(null);
  setShown({ place, items });
  // Over whatever it's asked for in, which stays until it closes.
  popup.open(KEY, { over: true });
}

/** Whether a menu is open, for what would come up over it to wait. */
export const menuOpen = () => popup.is(KEY);

/** Closes the menu, if one is open. */
export function closeMenu() {
  popup.close(KEY);
}

function Menus() {
  return (
    <Show when={shown()} keyed>
      {(menu) => {
        const place = menu.place;
        const node = "node" in place ? place : null;
        return (
          <Flyout
            output={place.output!}
            popup={KEY}
            from={node?.node}
            at={node ? undefined : (place as { x: number; y: number })}
            side={node ? node.side ?? "top" : "bottom"}
            align={node ? node.align ?? "center" : "start"}
            gap={node ? 6 : 4}
            padding={6}
            radius={12}
            keyboard
            // Over the shell, apart from its glass, but under other programs' overlays, like a
            // screenshot tool's, and the lock screen.
            layer="overlay"
            z={-1}
            onKey={(event) => keys.current?.(event)}
          >
            <Pages items={menu.items} />
          </Flyout>
        );
      }}
    </Show>
  );
}

/** What the menu that's open does with keys. */
const keys: { current: ((event: KeyEvent) => boolean) | null } = { current: null };

/** A menu's items, and its submenus in its place, one page at a time. */
function Pages(props: { items: MenuItem[] }) {
  // The pages open, the menu first, each with what it came from.
  const [pages, setPages] = createSignal<{ title?: string; items: MenuItem[] }[]>([
    { items: props.items },
  ]);
  const page = () => pages()[pages().length - 1];
  const [lit, setLit] = createSignal(-1);
  const usable = (item: MenuItem) => item !== "separator" && !item.disabled;

  const back = () => {
    if (pages().length < 2) return false;
    setPages(pages().slice(0, -1));
    setLit(-1);
    return true;
  };
  const use = (item: MenuItem) => {
    if (item === "separator" || item.disabled) return;
    if (item.items) {
      setPages([...pages(), { title: item.label, items: item.items }]);
      setLit(-1);
      return;
    }
    closeMenu();
    item.run?.();
  };
  // Up and Down go to the next item that can be used, around the ends.
  const move = (by: number) => {
    const items = page().items;
    if (!items.some(usable)) return;
    let at = lit();
    do at = (at + by + items.length) % items.length;
    while (!usable(items[at]));
    setLit(at);
  };
  keys.current = (event) => {
    if (!event.pressed) return false;
    const item = page().items[lit()];
    switch (event.key) {
      case "Down":
      case "Tab":
        move(1);
        return true;
      case "Up":
      case "ISO_Left_Tab":
        move(-1);
        return true;
      case "Return":
      case "KP_Enter":
      case "space":
        if (item) use(item);
        return true;
      case "Right":
        if (item && item !== "separator" && item.items) use(item);
        return true;
      case "Left":
      case "BackSpace":
        return back();
      default:
        return false;
    }
  };

  return (
    <box style={{ flexDirection: "column", gap: 2, width: WIDTH }}>
      <Show when={page().title} keyed>
        {(title) => (
          <>
            <Entry
              label={title}
              icon="go-previous-symbolic"
              strong
              lit={false}
              onUse={back}
              onHover={() => setLit(-1)}
            />
            <Separator />
          </>
        )}
      </Show>
      <For each={page().items}>
        {(item, index) =>
          item === "separator" ? (
            <Separator />
          ) : (
            <Entry
              label={item.label}
              icon={item.icon}
              checked={item.checked}
              danger={item.danger}
              disabled={item.disabled}
              more={!!item.items}
              lit={lit() === index()}
              onUse={() => use(item)}
              onHover={() => usable(item) && setLit(index())}
            />
          )
        }
      </For>
    </box>
  );
}

function Separator() {
  return <box style={{ height: 1, margin: [4, 8], background: "fg/0.12" }} />;
}

/** One item: its icon or check, what it says, and an arrow for a submenu. */
function Entry(props: {
  label: string;
  icon?: string;
  checked?: boolean;
  danger?: boolean;
  disabled?: boolean;
  more?: boolean;
  strong?: boolean;
  lit: boolean;
  onUse: () => void;
  onHover: () => void;
}) {
  const color = () => (props.danger ? "#e01b24" : "fg");
  return (
    <box
      style={{
        flexDirection: "row", alignItems: "center", gap: 10, minHeight: 32, padding: [0, 10],
        borderRadius: 8, opacity: props.disabled ? 0.45 : 1, color: color(),
        background: props.lit ? "fg/0.1" : undefined,
        ":hover": props.disabled ? undefined : { background: "fg/0.1" },
        cursor: props.disabled ? "default" : "pointer",
      }}
      onClick={props.disabled ? undefined : props.onUse}
      onHover={({ inside }) => inside && props.onHover()}
    >
      <box style={{ width: 16, height: 16, alignItems: "center", justifyContent: "center" }}>
        <Show when={props.checked ? "object-select-symbolic" : props.icon} keyed>
          {(icon) => <image icon={icon} size={16} />}
        </Show>
      </box>
      <text
        style={{
          flexGrow: 1, flexShrink: 1, minWidth: 0, lineClamp: 1,
          fontWeight: props.strong ? 700 : 400,
        }}
      >
        {props.label}
      </text>
      <Show when={props.more}>
        <image icon="go-next-symbolic" size={16} style={{ color: "fg/0.6" }} />
      </Show>
    </box>
  );
}
