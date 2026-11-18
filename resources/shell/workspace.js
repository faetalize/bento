// What the shell shows of workspaces besides their buttons: the minimap of the one on screen,
// and the editor that names one or takes its name away.

import {
  box, text, image, input, button, root, on, action, last, focus, focusedOutput, outputs, grow,
  shrink, opened, closed,
} from "bento:ui";

// Each output's workspaces: `{ index, name, active, focused, id }`.
let workspaces = last("workspaces")?.outputs ?? {};
on("workspaces", (event) => {
  workspaces = event.outputs;
});

const MAP_HEIGHT = 14;
// How wide the minimap gets: a workspace wider than that is squeezed in.
const MAP_WIDTH = 120;
// How much scrolling over the minimap moves the focus by one column.
const SCROLL_STEP = 15;

// The minimap of output `output`'s workspace: its tiles where they sit, with the part the view
// shows marked, and the focused one in the accent colour. Scrolling over it moves the focus
// along the columns, and a click on a tile focuses its window. Without tiles it takes no room.
export function minimap(output) {
  const view = box({
    position: "absolute", top: 0, left: 0, width: 0, height: MAP_HEIGHT, radius: 4,
    fill: "fg/0.16",
  });
  let scrolled = 0;
  // The tiles as last shown, to scroll along.
  let shown = [];
  const track = box(
    {
      height: MAP_HEIGHT, width: 0, display: "none", radius: 4, clip: true,
      onScroll: ({ dx, dy }) => {
        scrolled += dy || dx;
        if (Math.abs(scrolled) < SCROLL_STEP) return;
        const next = beside(shown, Math.sign(scrolled));
        if (next) action("focus-window", [], { window: next.id });
        scrolled = 0;
      },
    },
    [view],
  );
  const tiles = new Map();

  const show = (map) => {
    const list = (map?.tiles ?? []).filter((tile) => !tile.floating && tile.w > 0 && tile.h > 0);
    if (!map || list.length === 0 || !(map.view[0] > 0) || !(map.view[1] > 0)) {
      track.set({ display: "none" });
      return;
    }
    shown = list;
    // The canvas spans the tiles and the view, which starts at zero.
    const [viewWidth, viewHeight] = map.view;
    const start = Math.min(0, ...list.map((tile) => tile.x));
    const end = Math.max(viewWidth, ...list.map((tile) => tile.x + tile.w));
    const tall = MAP_HEIGHT / viewHeight;
    const wide = Math.min(tall, MAP_WIDTH / (end - start));
    track.set({ display: "flex", width: (end - start) * wide });
    view.set({ left: -start * wide, width: viewWidth * wide });

    const seen = new Set();
    for (const tile of list) {
      seen.add(tile.id);
      let node = tiles.get(tile.id);
      if (!node) {
        node = box({
          position: "absolute", radius: 2, cursor: "pointer",
          onClick: () => action("focus-window", [], { window: tile.id }),
        });
        track.add(node);
        tiles.set(tile.id, node);
      }
      const top = Math.min(Math.max(tile.y * tall, 0), MAP_HEIGHT - 2);
      // What's off the view is fainter than what's on it.
      const inView = tile.x + tile.w > 0 && tile.x < viewWidth;
      node.set({
        left: (tile.x - start) * wide + 1,
        top: top + 1,
        width: Math.max(tile.w * wide - 2, 2),
        height: Math.max(Math.min(tile.h * tall, MAP_HEIGHT - top) - 2, 2),
        fill: tile.active ? "accent" : inView ? "fg/0.55" : "fg/0.25",
      });
    }
    for (const [id, node] of tiles) {
      if (!seen.has(id)) {
        node.remove();
        tiles.delete(id);
      }
    }
  };

  show(last("minimap")?.outputs?.[output]);
  on("minimap", (event) => show(event.outputs?.[output]));
  return track;
}

// The tile next to the focused one in `tiles`, to the right with `side` 1 and to the left with
// -1: the nearest that starts further that way, and of those the one nearest its height. With
// none focused, the first or the last.
function beside(tiles, side) {
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

// The workspace editor: a field in the middle of the output with the name of its workspace,
// which is renamed as the name is typed. Its buttons take the name away, and put back the one it
// had when the editor opened.
const EDITOR_RADIUS = 24;
let editor = null;
// Each output's editor, kept while hidden.
const editors = new Map();
const panel = { close: () => closeEditor() };

function build(output) {
  const title = text("", { size: 16, weight: 700 });
  const field = input({
    placeholder: "Name",
    size: 17,
    onChange: (name) => rename(name),
    onSubmit: () => closeEditor(),
  });
  const error = text("", { size: 12, color: "#e5484d" });
  const undo = button({
    icon: "edit-undo-symbolic", label: "Undo", fill: "fg/0.08",
    onClick: () => field.setText(editor?.original ?? ""),
  });
  const remove = button({
    icon: "user-trash-symbolic", label: "Remove Name", fill: "fg/0.08",
    onClick: () => field.setText(""),
  });
  const done = button({ label: "Done", active: true, onClick: () => closeEditor() });
  // One body, so that it grows around the panel's middle as one.
  const body = box({ direction: "column", gap: 12, width: 380 }, [
    box({ align: "center", gap: 10 }, [
      image("document-edit-symbolic", { size: 18, color: "accent" }),
      title,
    ]),
    box({ padding: [9, 12], radius: 12, fill: "fg/0.08" }, [field]),
    box({ height: 16 }, [error]),
    box({ justify: "end", gap: 8 }, [undo, remove, done]),
  ]);
  const glass = box({ material: "glass", radius: EDITOR_RADIUS, padding: 18, clip: true }, [body]);
  const frame = box(
    {
      position: "absolute", top: 0, left: 0, right: 0, bottom: 0,
      direction: "column", align: "center", justify: "center",
    },
    [glass],
  );
  root(output)?.add(frame);
  return { output, glass, body, title, field, error, undo, remove };
}

// Whether another workspace is called `name` already: names are told apart without their case.
function taken(name, id) {
  const wanted = name.toLowerCase();
  return Object.values(workspaces)
    .flat()
    .find((ws) => ws.id !== id && (ws.name ?? "").toLowerCase() === wanted);
}

function rename(typed) {
  if (!editor) return;
  const name = typed.trim();
  const other = name && taken(name, editor.id);
  editor.error.set({ text: other ? `"${other.name}" is another workspace already.` : "" });
  editor.undo.set({ display: name === editor.original ? "none" : "flex" });
  editor.remove.set({ display: name ? "flex" : "none" });
  if (other || name === editor.name) return;
  editor.name = name;
  if (name) action("set-workspace-name", [name], { workspace: editor.id });
  else action("unset-workspace-name", [], { workspace: editor.id });
}

function openEditor(output) {
  const workspace = (workspaces[output] ?? []).find((ws) => ws.active);
  if (!workspace || !root(output)) return;
  opened(panel);
  if (!editors.has(output)) editors.set(output, build(output));
  const original = workspace.name ?? "";
  editor = { ...editors.get(output), id: workspace.id, original, name: original };
  editor.title.set({ text: `Workspace ${workspace.index + 1}` });
  editor.field.setText(original);
  editor.error.set({ text: "" });
  editor.undo.set({ display: "none" });
  editor.remove.set({ display: original ? "flex" : "none" });
  focus(editor.field);
  grow(editor.glass, { radius: EDITOR_RADIUS, content: [editor.body] });
}

function closeEditor() {
  if (!editor) return;
  shrink(editor.glass, { content: [editor.body] });
  editor = null;
  closed(panel);
}

on("edit-workspace", (event) => {
  if (editor) closeEditor();
  else openEditor(event.output ?? focusedOutput() ?? outputs()[0]);
});
