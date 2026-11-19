// The tray: apps' status icons, as they give them over StatusNotifierItem. The pinned ones show in
// a row, and the rest behind a chevron, in a flyout. A click activates an icon, a middle click does
// its other thing, and a right click opens its app's menu, in bento's own menu, with a way to pin
// it to the tray or take it off.

import {
  command, data, onData, useModule, type Data, type Edge, type Meta, type OutputName,
  type TrayMenuItem,
} from "bento";
import { For, Show } from "solid-js";
import { Flyout, popup } from "bento:modules/ui";
import { openMenu, type MenuItem } from "bento:modules/menu";

export const meta = {
  settings: {
    pinned: {
      type: "string",
      list: true,
      default: [] as string[],
      description:
        "The status icons that show in the tray, by their app's name for them, like nm-applet. " +
        "The rest are behind its chevron.",
    },
  },
} satisfies Meta;

export const { settings, setSetting } = useModule(import.meta, meta);

export type TrayIcon = Data["tray"]["items"][number];

export const trayItems = (): TrayIcon[] => data("tray")?.items ?? [];

export const isPinned = (item: TrayIcon) => settings.pinned.includes(item.name);

/** Pins `item` to the tray, at its end, or takes it off, behind the chevron. */
export function pin(item: TrayIcon, pinned: boolean) {
  const list = settings.pinned.filter((name) => name !== item.name);
  if (pinned) list.push(item.name);
  setSetting("pinned", list);
}

// Where each menu that was asked for opens, by its item, until the data program sends it.
const asked = new Map<string, { output: OutputName; x: number; y: number }>();

/** Asks for `item`'s menu, to open at the point `at`. */
function askMenu(item: TrayIcon, at: { output?: OutputName; x: number; y: number }, output: string) {
  asked.set(item.id, { output: at.output ?? output, x: at.x, y: at.y });
  command("tray-menu", { id: item.id });
}

/** An app's menu as bento's: its separators where they part items, never at an end. */
function menuOf(id: string, nodes: TrayMenuItem[]): MenuItem[] {
  const items: MenuItem[] = [];
  for (const node of nodes) {
    if (node.separator) {
      if (items.length > 0 && items[items.length - 1] !== "separator") items.push("separator");
      continue;
    }
    items.push({
      label: node.label,
      disabled: !node.enabled,
      checked: node.toggle !== null && node.on,
      items: node.children.length > 0 ? menuOf(id, node.children) : undefined,
      run: () => command("tray-menu-click", { id, item: node.id }),
    });
  }
  if (items[items.length - 1] === "separator") items.pop();
  return items;
}

onData("tray-menu", ({ id, menu }) => {
  const place = asked.get(id);
  asked.delete(id);
  const item = trayItems().find((one) => one.id === id);
  if (!place || !item) return;
  const own = menuOf(id, menu);
  const pinning: MenuItem = isPinned(item)
    ? { label: "Unpin from Tray", icon: "view-pin-symbolic", run: () => pin(item, false) }
    : { label: "Pin to Tray", icon: "view-pin-symbolic", run: () => pin(item, true) };
  openMenu(place, own.length > 0 ? [...own, "separator", pinning] : [pinning]);
});

/** One status icon. */
function Icon(props: { item: TrayIcon; output: string; size: number }) {
  const menu = (event: { output?: OutputName; x: number; y: number }) =>
    askMenu(props.item, event, props.output);
  return (
    <box
      style={{
        width: 28, height: 28, borderRadius: 6, alignItems: "center", justifyContent: "center",
        ":hover": { background: "fg/0.08" },
      }}
      onClick={(event) =>
        props.item.menu ? menu(event) : command("activate", { id: props.item.id })}
      onRightClick={menu}
      onMiddleClick={() => command("secondary", { id: props.item.id })}
    >
      <image icon={props.item.icon ?? "application-x-executable-symbolic"} size={props.size} />
    </box>
  );
}

/** The tray on `output`: the pinned icons in a row, and a chevron to the rest, which come out
 * on its `side`, `"top"` for a tray along the bottom of the output, `gap` from it. */
export function Tray(props: {
  output: string;
  side?: Edge;
  gap?: number;
  size?: number;
  style?: any;
}) {
  const key = () => `tray:${props.output}`;
  const side = () => props.side ?? "bottom";
  const size = () => props.size ?? 16;
  const shown = () => trayItems().filter(isPinned);
  const hidden = () => trayItems().filter((item) => !isPinned(item));
  let chevron: any;
  return (
    <box style={[{ flexDirection: "row", alignItems: "center", gap: 2 }, props.style]}>
      <Show when={hidden().length > 0}>
        <box
          ref={chevron}
          style={{
            width: 28, height: 28, borderRadius: 6, alignItems: "center", justifyContent: "center",
            background: popup.is(key()) ? "fg/0.1" : undefined, ":hover": { background: "fg/0.08" },
          }}
          onClick={() => popup.toggle(key())}
        >
          {/* A chevron towards where the rest comes out, which turns back while it's out. */}
          <image
            icon={(side() === "top") !== popup.is(key()) ? "pan-up-symbolic" : "pan-down-symbolic"}
            size={size()}
          />
        </box>
        <Flyout
          output={props.output}
          popup={key()}
          from={chevron}
          side={side()}
          gap={props.gap}
          padding={6}
          radius={12}
        >
          <box style={{ flexDirection: "row", flexWrap: "wrap", gap: 2, maxWidth: 5 * 30 }}>
            <For each={hidden()} keyed={(item: TrayIcon) => item.id}>
              {(item: () => TrayIcon) => <Icon item={item()} output={props.output} size={size()} />}
            </For>
          </box>
        </Flyout>
      </Show>
      <For each={shown()} keyed={(item: TrayIcon) => item.id}>
        {(item: () => TrayIcon) => <Icon item={item()} output={props.output} size={size()} />}
      </For>
    </box>
  );
}
