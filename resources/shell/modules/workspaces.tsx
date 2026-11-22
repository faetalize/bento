// The workspaces of an output: their buttons, as numbers or names, the current one lit, and the
// minimap of the one on screen.

import { action, outputSize, useOutput, windows, workspaces, type Workspace } from "bento";
import { createMemo, For, onCleanup, Show } from "solid-js";
import { openMenu, type MenuItem } from "bento:modules/menu";
import { ACTIVE } from "bento:modules/ui";

/** What a workspace is called: its name, or which it is on its output. */
export const workspaceName = (ws: Workspace) => ws.name ?? `Workspace ${ws.index + 1}`;

// The editor the shell placed, if it did. It's handed over here, rather than imported, so that
// showing workspaces doesn't bring an editor and its action into a shell that has none, or its own.
let editor: ((ws: Workspace) => void) | null = null;

/** Has `edit` open the shell's workspace editor, while the component running now is placed. */
export function provideWorkspaceEditor(edit: (ws: Workspace) => void) {
  editor = edit;
  onCleanup(() => {
    if (editor === edit) editor = null;
  });
}

/** Whether the shell placed a workspace editor. */
export const canEditWorkspaces = () => editor !== null;

/** Opens the shell's workspace editor for `ws`, if it placed one. */
export const openWorkspaceEditor = (ws: Workspace) => editor?.(ws);

/** What a right click on a workspace offers: renaming it, where the shell has an editor. */
export function workspaceMenu(ws: Workspace): MenuItem[] {
  if (!canEditWorkspaces()) return [];
  return [
    { label: "Rename Workspace", icon: "document-edit-symbolic", run: () => openWorkspaceEditor(ws) },
  ];
}

/** The output's workspaces, a click going to one and a right click opening its menu. */
export function Workspaces() {
  const output = useOutput();
  return (
    <box style={{ flexDirection: "row", gap: 2 }}>
      <For each={workspaces(output)}>
        {(ws) => (
          <box
            style={{
              flexDirection: "row", minWidth: 26, height: 26, borderRadius: 13,
              padding: [0, ws.name ? 9 : 0], alignItems: "center", justifyContent: "center",
              background: ws.active ? ACTIVE : undefined, cursor: "pointer",
              ":hover": ws.active ? undefined : { background: "fg/0.12" },
            }}
            onClick={() => action("focus-workspace", [], { workspace: ws.id })}
            onRightClick={(event) => {
              const items = workspaceMenu(ws);
              if (items.length > 0) openMenu(event, items);
            }}
          >
            <text
              style={{
                fontWeight: 700, fontVariantNumeric: "tabular-nums",
                color: ws.active ? "#ffffff" : "fg",
              }}
            >
              {ws.name ?? String(ws.index + 1)}
            </text>
          </box>
        )}
      </For>
    </box>
  );
}

const MAP_HEIGHT = 14;
// How wide the minimap gets: a workspace wider than that is squeezed in.
const MAP_WIDTH = 120;
// How much scrolling over the minimap moves the focus by one column.
const SCROLL_STEP = 15;

/** The tile next to the focused one in `tiles`, to the right with `side` 1 and to the left with
 * -1: the nearest that starts further that way, and of those the one nearest its height. With
 * none focused, the first or the last. */
function beside(tiles: any[], side: number) {
  const active = tiles.find((tile) => tile.active);
  const sorted = [...tiles].sort((a, b) => a.x - b.x || a.y - b.y);
  if (!active) return side > 0 ? sorted[0] : sorted[sorted.length - 1];
  const further = sorted.filter((tile) => (tile.x - active.x) * side > 0.5);
  if (further.length === 0) return null;
  const column = side > 0 ? further[0].x : further[further.length - 1].x;
  return further
    .filter((tile) => Math.abs(tile.x - column) < 0.5)
    .sort((a, b) => Math.abs(a.y - active.y) - Math.abs(b.y - active.y))[0];
}

/** The output's workspace as a track: its tiles where they sit, with the part the view shows
 * marked, and the focused one lit. Scrolling over it moves the focus along the columns, and a
 * click on a tile focuses its window. Without tiles it takes no room. */
export function Minimap() {
  const output = useOutput();
  const tiles = createMemo(() => {
    const shown = workspaces(output).find((ws) => ws.active);
    return windows()
      .filter((win) => win.workspace === shown?.id && win.rect && !win.floating)
      .map((win) => ({
        id: win.id, active: win.active,
        x: win.rect!.x, y: win.rect!.y, w: win.rect!.width, h: win.rect!.height,
      }))
      .filter((tile) => tile.w > 0 && tile.h > 0);
  });
  const shape = createMemo(() => {
    const view = outputSize(output);
    const list = tiles();
    if (!view || list.length === 0 || !(view.width > 0) || !(view.height > 0)) return null;
    // The canvas spans the tiles and the view, which starts at zero.
    const { width: viewWidth, height: viewHeight } = view;
    const start = Math.min(0, ...list.map((tile: any) => tile.x));
    const end = Math.max(viewWidth, ...list.map((tile: any) => tile.x + tile.w));
    const tall = MAP_HEIGHT / viewHeight;
    const wide = Math.min(tall, MAP_WIDTH / (end - start));
    return { start, tall, wide, viewWidth, width: (end - start) * wide };
  });
  let scrolled = 0;
  return (
    <Show when={shape()}>
      <box
        style={{
          height: MAP_HEIGHT, width: shape()!.width, borderRadius: 4, overflow: "hidden",
        }}
        onScroll={({ dx, dy }) => {
          scrolled += dy || dx;
          if (Math.abs(scrolled) < SCROLL_STEP) return;
          const next = beside(tiles(), Math.sign(scrolled));
          if (next) action("focus-window", [], { window: next.id });
          scrolled = 0;
        }}
      >
        <box
          style={{
            position: "absolute", top: 0, height: MAP_HEIGHT, borderRadius: 4, background: "fg/0.16",
            left: -shape()!.start * shape()!.wide, width: shape()!.viewWidth * shape()!.wide,
          }}
        />
        <For each={tiles()}>
          {(tile: any) => {
            const top = () => Math.min(Math.max(tile.y * shape()!.tall, 0), MAP_HEIGHT - 2);
            // What's off the view is fainter than what's on it.
            const inView = () => tile.x + tile.w > 0 && tile.x < shape()!.viewWidth;
            return (
              <box
                style={{
                  position: "absolute", borderRadius: 2, cursor: "pointer",
                  left: (tile.x - shape()!.start) * shape()!.wide + 1,
                  top: top() + 1,
                  width: Math.max(tile.w * shape()!.wide - 2, 2),
                  height: Math.max(Math.min(tile.h * shape()!.tall, MAP_HEIGHT - top()) - 2, 2),
                  background: tile.active ? "accent" : inView() ? "fg/0.55" : "fg/0.25",
                }}
                onClick={() => action("focus-window", [], { window: tile.id })}
              />
            );
          }}
        </For>
      </box>
    </Show>
  );
}
