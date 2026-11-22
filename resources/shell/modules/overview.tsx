// The overview's button for a taskbar: an icon, filled while the overview is open, with a pip for
// each workspace of its output and the current one's name. A click opens the overview, scrolling
// over it goes through the workspaces, resting on it shows previews of them all, and a right click
// opens the current one's menu.

import {
  action, after, outputSize, overviewOpen, useModule, windows, workspaces, type Meta, type Workspace,
} from "bento";
import { createMemo, createSignal, For, Show } from "solid-js";
import { Flyout } from "bento:modules/ui";
import { menuOpen, openMenu } from "bento:modules/menu";
import { workspaceMenu, workspaceName } from "bento:modules/workspaces";

export const meta = {
  settings: {
    "workspace-names": {
      type: "boolean",
      default: true,
      description: "Show the current workspace's name, or which it is when it has none.",
      page: "shell",
      group: "Overview Button",
    },
    "workspace-number": {
      type: "boolean",
      default: false,
      description: "Show the current workspace's number in the icon.",
      page: "shell",
      group: "Overview Button",
    },
    "workspace-pips": {
      type: "boolean",
      default: true,
      description: "Show a pip for each workspace, the current one's lit.",
      page: "shell",
      group: "Overview Button",
    },
    "hover-previews": {
      type: "boolean",
      default: true,
      description: "Show previews of the workspaces while the pointer rests on the button.",
      page: "shell",
      group: "Overview Button",
    },
    "scroll-to-switch": {
      type: "boolean",
      default: true,
      description: "Go through the workspaces by scrolling over the button.",
      page: "shell",
      group: "Overview Button",
    },
  },
} satisfies Meta;

export const { settings } = useModule(import.meta, meta);

const ICON = 20;
// The front card in the icon, where the number goes: its top and height.
const CARD = { top: (104 / 256) * ICON, height: (96 / 256) * ICON };
// How wide a name gets before it's cut.
const NAME = 160;
// A pip's size, the current one's length, the room between two, and how many show.
const PIP = 3;
const LONG = 10;
const PIP_GAP = 3;
const PIPS = 5;
const PIPS_HEIGHT = PIPS * (PIP + PIP_GAP) - PIP_GAP + LONG - PIP;
// How long the pointer rests on the button before the previews show, how long they stay after it
// leaves, and how long after a scroll over it they're back, in milliseconds.
const REST = 400;
const LINGER = 300;
const QUIET = 600;
// How much scrolling goes one workspace on: a wheel's notch.
const SCROLL_STEP = 15;
// How wide a preview is, how many are in a row, and the room around and between them.
const PREVIEW = 180;
const ROW = 4;
const ROOM = 6;

/** The overview's button for the taskbar on `output`, `size` tall, its previews `gap` away from it
 * and lined up with it by `align`. */
export function OverviewButton(props: {
  output: string;
  size?: number;
  gap?: number;
  align?: "start" | "center" | "end";
}) {
  const size = () => props.size ?? 40;
  const list = () => workspaces(props.output);
  const at = createMemo(() => Math.max(0, list().findIndex((ws) => ws.active)));
  const current = () => list()[at()];
  const [button, setButton] = createSignal<any>(null);
  const [open, setOpen] = createSignal(false);

  let timer: (() => void) | null = null;
  const later = (ms: number, fn: () => void) => {
    timer?.();
    timer = after(ms, fn);
  };
  let over = false;
  // Not over a menu, nor over the overview, which shows the workspaces itself.
  const show = (ms: number) =>
    later(ms, () => over && settings["hover-previews"] && !menuOpen() && !overviewOpen() && setOpen(true));
  const hide = () => later(LINGER, () => setOpen(false));
  const close = () => {
    timer?.();
    setOpen(false);
  };

  // How wide it stays at least while it's scrolled over, so that it doesn't go from under the
  // pointer as it goes from a long name to a short one.
  const [held, setHeld] = createSignal(0);
  let scrolled = 0;
  const scroll = (by: number) => {
    if (!settings["scroll-to-switch"]) return;
    setHeld(Math.max(held(), button()?.layout()?.width ?? 0));
    // The workspaces go by on screen, so the previews keep out of the way until it's done.
    close();
    show(QUIET);
    scrolled += by;
    if (Math.abs(scrolled) < SCROLL_STEP) return;
    const next = list()[at() + Math.sign(scrolled)];
    scrolled = 0;
    if (next) action("focus-workspace", [], { workspace: next.id });
  };

  return (
    <>
      <box
        ref={setButton}
        style={{
          flexDirection: "row", alignItems: "center", gap: 8, height: size(),
          minWidth: Math.max(size(), held()), padding: [0, (size() - ICON) / 2], borderRadius: 8,
          flexShrink: 0,
          background: overviewOpen() ? "fg/0.1" : undefined, ":hover": { background: "fg/0.14" },
        }}
        onClick={() => {
          close();
          action("toggle-overview");
        }}
        onRightClick={() => {
          const ws = current();
          const items = ws ? workspaceMenu(ws) : [];
          if (items.length === 0) return;
          close();
          openMenu({ output: props.output, node: button(), side: "top" }, items);
        }}
        onScroll={({ dx, dy }) => scroll(dy || dx)}
        onNear={({ inside }) => {
          over = inside;
          if (!inside) {
            scrolled = 0;
            setHeld(0);
          }
          inside ? show(open() ? 0 : REST) : hide();
        }}
      >
        <box style={{ width: ICON, height: ICON }}>
          <image icon="ph-cards-three-symbolic" size={ICON} />
          <image
            icon="ph-cards-three-fill-symbolic"
            size={ICON}
            style={{ position: "absolute", left: 0, top: 0, opacity: overviewOpen() ? 1 : 0 }}
          />
          <Show when={settings["workspace-number"] && current()}>
            <box
              style={{
                position: "absolute", left: 0, top: CARD.top, width: ICON, height: CARD.height,
                alignItems: "center", justifyContent: "center",
              }}
            >
              <text
                style={{
                  fontSize: 8, fontWeight: 700, fontVariantNumeric: "tabular-nums", flexShrink: 0,
                  color: overviewOpen() ? "window" : "fg",
                }}
              >
                {String(at() + 1)}
              </text>
            </box>
          </Show>
        </box>
        <Show when={settings["workspace-pips"]}>
          <Pips list={list()} at={at()} />
        </Show>
        {/* As wide as the name: as that changes, the name is whole at once and the box uncovers
            it. */}
        <Show when={settings["workspace-names"] && current()}>
          <box style={{ overflow: "hidden", maxWidth: NAME, flexShrink: 0 }}>
            <text style={{ fontWeight: 600, lineClamp: 1 }}>{workspaceName(current())}</text>
          </box>
        </Show>
      </box>
      <Flyout
        output={props.output}
        open={open()}
        from={button()}
        side="top"
        align={props.align}
        gap={props.gap}
        padding={8}
        light={false}
      >
        <box
          style={{
            flexDirection: "row", flexWrap: "wrap",
            width: Math.min(ROW, list().length) * (PREVIEW + 2 * ROOM),
          }}
          onNear={({ inside }) => (inside ? timer?.() : hide())}
        >
          <For each={list()}>
            {(ws) => (
              <Preview
                output={props.output}
                workspace={ws}
                onClick={() => {
                  action("focus-workspace", [], { workspace: ws.id });
                  close();
                }}
              />
            )}
          </For>
        </box>
      </Flyout>
    </>
  );
}

/** A pip for each workspace, from the top, the current one long and lit. No more than `PIPS` show,
 * around the current one, and one at an end with more past it is smaller. */
function Pips(props: { list: Workspace[]; at: number }) {
  const first = () => Math.max(0, Math.min(props.list.length - PIPS, props.at - Math.floor(PIPS / 2)));
  const shown = () => Math.min(props.list.length, PIPS);
  const top = () => (PIPS_HEIGHT - (shown() * (PIP + PIP_GAP) - PIP_GAP + LONG - PIP)) / 2;
  return (
    <box style={{ width: PIP, height: PIPS_HEIGHT, overflow: "hidden" }}>
      <For each={props.list}>
        {(ws, index) => {
          const place = () => index() - first();
          const hidden = () => place() < 0 || place() >= PIPS;
          const edge = () =>
            (place() === 0 && first() > 0) ||
            (place() === PIPS - 1 && index() < props.list.length - 1);
          return (
            <box
              style={{
                position: "absolute", left: 0, width: PIP, borderRadius: PIP / 2,
                height: ws.active ? LONG : PIP,
                top: top() + place() * (PIP + PIP_GAP) + (index() > props.at ? LONG - PIP : 0),
                background: ws.active ? "accent" : "fg/0.4",
                opacity: hidden() ? 0 : edge() ? 0.7 : 1,
                scale: edge() ? 0.67 : 1,
              }}
              enter={{ opacity: 0 }}
              exit={{ opacity: 0 }}
            />
          );
        }}
      </For>
    </box>
  );
}

/** A workspace, small: its name over its windows, live, where they are on it. */
function Preview(props: { output: string; workspace: Workspace; onClick: () => void }) {
  const view = () => outputSize(props.output) ?? { width: 16, height: 9 };
  const scale = () => PREVIEW / view().width;
  // From the back to the front, as later ones are drawn over earlier ones.
  const shown = createMemo(() =>
    windows().filter((win) => win.workspace === props.workspace.id && win.rect).reverse());
  return (
    <box
      style={{
        flexDirection: "column", gap: ROOM, padding: ROOM, borderRadius: 12,
        width: PREVIEW + 2 * ROOM,
        background: props.workspace.active ? "accent/0.25" : undefined,
        ":hover": props.workspace.active ? undefined : { background: "fg/0.1" },
      }}
      onClick={props.onClick}
    >
      <text style={{ fontSize: 12, lineClamp: 1 }}>{workspaceName(props.workspace)}</text>
      <box
        style={{
          width: PREVIEW, height: Math.round(view().height * scale()), borderRadius: 6,
          overflow: "hidden", background: "fg/0.08",
        }}
      >
        <For each={shown()}>
          {(win) => (
            <copy
              window={win.id}
              style={{
                position: "absolute", borderRadius: 3,
                left: win.rect!.x * scale(), top: win.rect!.y * scale(),
                width: win.rect!.width * scale(), height: win.rect!.height * scale(),
              }}
            />
          )}
        </For>
      </box>
    </box>
  );
}
